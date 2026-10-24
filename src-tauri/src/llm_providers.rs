//! Per-runtime LLM endpoint routing through ACP `providers/*` (unstable). A saved config makes
//! an adapter send its model traffic to the user's own endpoint.
//!
//! Provider settings live in the adapter process, so every fresh connection gets them again:
//! `apply_to_connection` runs right after `initialize`, before any session call. Header values
//! may carry API keys, so SQLite keeps only header names; the values live in the OS keychain
//! (`credentials::secrets`) and are read when a connection starts.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::acp::protocol as acp;
use crate::acp::session::list::probe_runtime;
use crate::db::{get_state, with_db};
use crate::runtime_kind::RuntimeKind;
use crate::types::AppState;

pub(crate) fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_provider_configs (
           runtime_key TEXT NOT NULL,
           provider_id TEXT NOT NULL,
           api_type TEXT NOT NULL,
           base_url TEXT NOT NULL,
           header_names_json TEXT NOT NULL DEFAULT '[]',
           updated_at INTEGER NOT NULL,
           PRIMARY KEY(runtime_key, provider_id)
         );",
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredConfig {
    provider_id: String,
    api_type: String,
    base_url: String,
    header_names: Vec<String>,
}

/// Non-secret rows, mirrored in memory so the ACP worker thread (which has no `AppState`) can
/// read them when a connection starts.
fn snapshot() -> &'static Mutex<HashMap<String, Vec<StoredConfig>>> {
    static S: OnceLock<Mutex<HashMap<String, Vec<StoredConfig>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Last apply failure per runtime, shown next to the form.
fn apply_errors() -> &'static Mutex<HashMap<String, String>> {
    static S: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn revisions() -> &'static Mutex<HashMap<String, u64>> {
    static S: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

static REVISION_SEQ: AtomicU64 = AtomicU64::new(1);

/// Changes whenever the runtime's saved config changes. A live connection remembers the value
/// it started with; a mismatch means it was configured with stale settings and is evicted.
pub(crate) fn revision(runtime_key: &str) -> u64 {
    revisions()
        .lock()
        .ok()
        .and_then(|m| m.get(runtime_key).copied())
        .unwrap_or(0)
}

fn bump_revision(runtime_key: &str) {
    if let Ok(mut m) = revisions().lock() {
        m.insert(
            runtime_key.to_string(),
            REVISION_SEQ.fetch_add(1, Ordering::Relaxed),
        );
    }
}

fn secret_id(runtime_key: &str, provider_id: &str) -> String {
    format!("llm-provider:{runtime_key}:{provider_id}")
}

fn read_rows(conn: &Connection) -> Result<HashMap<String, Vec<StoredConfig>>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT runtime_key, provider_id, api_type, base_url, header_names_json
             FROM llm_provider_configs ORDER BY runtime_key, provider_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let names: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                StoredConfig {
                    provider_id: row.get(1)?,
                    api_type: row.get(2)?,
                    base_url: row.get(3)?,
                    header_names: serde_json::from_str(&names).unwrap_or_default(),
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out: HashMap<String, Vec<StoredConfig>> = HashMap::new();
    for row in rows {
        let (runtime, cfg) = row.map_err(|e| e.to_string())?;
        out.entry(runtime).or_default().push(cfg);
    }
    Ok(out)
}

pub(crate) fn load_snapshot(conn: &Connection) -> Result<(), String> {
    let rows = read_rows(conn)?;
    if let Ok(mut s) = snapshot().lock() {
        *s = rows;
    }
    Ok(())
}

fn stored_for(runtime_key: &str) -> Vec<StoredConfig> {
    snapshot()
        .lock()
        .ok()
        .and_then(|s| s.get(runtime_key).cloned())
        .unwrap_or_default()
}

fn parse_protocol(raw: &str) -> acp::LlmProtocol {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .unwrap_or_else(|_| acp::LlmProtocol::Other(raw.to_string()))
}

fn protocol_name(p: &acp::LlmProtocol) -> String {
    match serde_json::to_value(p) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// A header whose value is missing from the keychain is dropped rather than sent empty: an
/// empty `Authorization` would fail upstream with a less useful error than "no key".
fn build_request(
    cfg: &StoredConfig,
    secrets: &BTreeMap<String, String>,
) -> acp::SetProviderRequest {
    let headers = cfg
        .header_names
        .iter()
        .filter_map(|name| secrets.get(name).map(|v| (name.clone(), v.clone())))
        .collect::<HashMap<_, _>>();
    acp::SetProviderRequest::new(
        cfg.provider_id.clone(),
        parse_protocol(&cfg.api_type),
        cfg.base_url.clone(),
    )
    .headers(headers)
}

/// The `providers/set` calls a freshly initialized adapter needs, one per saved provider; a
/// provider whose secret cannot be read is an `Err` entry so the caller can report it.
pub(crate) fn pending_requests(
    runtime_key: &str,
) -> Vec<(String, Result<acp::SetProviderRequest, String>)> {
    stored_for(runtime_key)
        .into_iter()
        .map(|cfg| {
            let secrets =
                crate::credentials::secrets::load(&secret_id(runtime_key, &cfg.provider_id));
            let req = secrets.map(|s| build_request(&cfg, &s));
            (cfg.provider_id, req)
        })
        .collect()
}

/// Keeps the apply outcome for the settings page; the connection itself never fails on it.
pub(crate) fn record_apply(runtime_key: &str, failures: &[String]) {
    if let Ok(mut m) = apply_errors().lock() {
        if failures.is_empty() {
            m.remove(runtime_key);
        } else {
            m.insert(runtime_key.to_string(), failures.join("; "));
        }
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveProvider {
    provider_id: String,
    supported: Vec<String>,
    required: bool,
    current_api_type: Option<String>,
    current_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedProviderConfig {
    provider_id: String,
    api_type: String,
    base_url: String,
    /// Names only; values never leave the keychain once saved.
    header_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedProviders {
    configs: Vec<SavedProviderConfig>,
    apply_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeaderInput {
    name: String,
    /// `None` keeps the value already in the keychain under this name.
    value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderConfigInput {
    provider_id: String,
    api_type: String,
    base_url: String,
    headers: Vec<HeaderInput>,
}

fn runtime_kind(runtime_key: &str) -> Result<RuntimeKind, String> {
    RuntimeKind::from_str(runtime_key).ok_or_else(|| format!("unknown runtime: {runtime_key}"))
}

fn validate(input: &ProviderConfigInput) -> Result<(), String> {
    if input.provider_id.trim().is_empty() {
        return Err("provider id is required".into());
    }
    if input.api_type.trim().is_empty() {
        return Err("protocol is required".into());
    }
    let url = input.base_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("base URL must start with http:// or https://".into());
    }
    let mut seen = std::collections::HashSet::new();
    for h in &input.headers {
        let name = h.name.trim();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() && b != b':') {
            return Err(format!("invalid header name: {name:?}"));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(format!("duplicate header: {name}"));
        }
    }
    Ok(())
}

/// Secrets after a save: submitted values win, an omitted value keeps the stored one, and
/// names no longer listed are dropped.
fn merge_secrets(
    existing: &BTreeMap<String, String>,
    headers: &[HeaderInput],
) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|h| {
            let name = h.name.trim().to_string();
            match &h.value {
                Some(v) if !v.is_empty() => Some((name, v.clone())),
                _ => existing.get(&name).map(|v| (name, v.clone())),
            }
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn list_runtime_providers_cmd(
    runtime_key: String,
) -> Result<Option<Vec<LiveProvider>>, String> {
    let kind = runtime_kind(&runtime_key)?;
    tokio::task::spawn_blocking(move || {
        probe_runtime(kind, "providers/list", |conn, init| async move {
            if init.agent_capabilities.providers.is_none() {
                return Ok(None);
            }
            let resp: acp::ListProvidersResponse = conn
                .request("providers/list", acp::ListProvidersRequest::new())
                .await
                .map_err(|e| format!("providers/list: {e}"))?;
            Ok(Some(
                resp.providers
                    .into_iter()
                    .map(|p| LiveProvider {
                        provider_id: p.provider_id.to_string(),
                        supported: p.supported.iter().map(protocol_name).collect(),
                        required: p.required,
                        current_api_type: p.current.as_ref().map(|c| protocol_name(&c.api_type)),
                        current_base_url: p.current.map(|c| c.base_url),
                    })
                    .collect(),
            ))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn get_llm_provider_configs_cmd(runtime_key: String) -> SavedProviders {
    let configs = stored_for(&runtime_key)
        .into_iter()
        .map(|c| SavedProviderConfig {
            provider_id: c.provider_id,
            api_type: c.api_type,
            base_url: c.base_url,
            header_names: c.header_names,
        })
        .collect();
    let apply_error = apply_errors()
        .lock()
        .ok()
        .and_then(|m| m.get(&runtime_key).cloned());
    SavedProviders {
        configs,
        apply_error,
    }
}

fn changed(runtime_key: &str, conn: &Connection) -> Result<(), String> {
    load_snapshot(conn)?;
    if let Ok(mut m) = apply_errors().lock() {
        m.remove(runtime_key);
    }
    bump_revision(runtime_key);
    Ok(())
}

#[tauri::command]
pub(crate) fn set_llm_provider_config_cmd(
    state: State<'_, AppState>,
    runtime_key: String,
    config: ProviderConfigInput,
) -> Result<(), String> {
    runtime_kind(&runtime_key)?;
    validate(&config)?;
    let id = secret_id(&runtime_key, config.provider_id.trim());
    let existing = crate::credentials::secrets::load(&id)?;
    let secrets = merge_secrets(&existing, &config.headers);
    crate::credentials::secrets::save(&id, &secrets)?;
    let names: Vec<String> = config
        .headers
        .iter()
        .map(|h| h.name.trim().to_string())
        .collect();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO llm_provider_configs
               (runtime_key, provider_id, api_type, base_url, header_names_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))
             ON CONFLICT(runtime_key, provider_id) DO UPDATE SET
               api_type = excluded.api_type, base_url = excluded.base_url,
               header_names_json = excluded.header_names_json, updated_at = excluded.updated_at",
            params![
                runtime_key,
                config.provider_id.trim(),
                config.api_type.trim(),
                config.base_url.trim(),
                serde_json::to_string(&names).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        changed(&runtime_key, conn)
    })
}

#[tauri::command]
pub(crate) fn clear_llm_provider_config_cmd(
    state: State<'_, AppState>,
    runtime_key: String,
    provider_id: String,
) -> Result<(), String> {
    crate::credentials::secrets::delete(&secret_id(&runtime_key, &provider_id))?;
    with_db(get_state(&state), |conn| {
        conn.execute(
            "DELETE FROM llm_provider_configs WHERE runtime_key = ?1 AND provider_id = ?2",
            params![runtime_key, provider_id],
        )
        .map_err(|e| e.to_string())?;
        changed(&runtime_key, conn)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(headers: Vec<(&str, Option<&str>)>) -> ProviderConfigInput {
        ProviderConfigInput {
            provider_id: "main".into(),
            api_type: "anthropic".into(),
            base_url: "https://llm.example.com".into(),
            headers: headers
                .into_iter()
                .map(|(n, v)| HeaderInput {
                    name: n.into(),
                    value: v.map(Into::into),
                })
                .collect(),
        }
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(validate(&input(vec![])).is_ok());
        let mut bad = input(vec![]);
        bad.base_url = "ftp://x".into();
        assert!(validate(&bad).is_err());
        assert!(validate(&input(vec![("A b", Some("x"))])).is_err());
        assert!(validate(&input(vec![("X-Key", None), ("x-key", None)])).is_err());
    }

    #[test]
    fn merge_keeps_omitted_values_and_drops_removed_names() {
        let existing = BTreeMap::from([
            ("Authorization".to_string(), "old".to_string()),
            ("X-Gone".to_string(), "g".to_string()),
        ]);
        let merged = merge_secrets(
            &existing,
            &input(vec![("Authorization", None), ("X-New", Some("n"))]).headers,
        );
        assert_eq!(merged.get("Authorization").map(String::as_str), Some("old"));
        assert_eq!(merged.get("X-New").map(String::as_str), Some("n"));
        assert!(!merged.contains_key("X-Gone"));
        let replaced = merge_secrets(
            &existing,
            &input(vec![("Authorization", Some("new"))]).headers,
        );
        assert_eq!(replaced["Authorization"], "new");
    }

    #[test]
    fn request_carries_protocol_url_and_present_headers_only() {
        let cfg = StoredConfig {
            provider_id: "main".into(),
            api_type: "openai".into(),
            base_url: "https://x".into(),
            header_names: vec!["Authorization".into(), "Missing".into()],
        };
        let secrets = BTreeMap::from([("Authorization".to_string(), "Bearer k".to_string())]);
        let req = build_request(&cfg, &secrets);
        assert_eq!(req.api_type, acp::LlmProtocol::OpenAi);
        assert_eq!(req.headers.len(), 1);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["providerId"], "main");
        assert_eq!(v["apiType"], "openai");
        assert_eq!(v["baseUrl"], "https://x");
    }

    #[test]
    fn protocols_round_trip_including_custom() {
        assert_eq!(protocol_name(&parse_protocol("anthropic")), "anthropic");
        assert_eq!(protocol_name(&parse_protocol("_acme")), "_acme");
    }

    #[test]
    fn schema_round_trips_through_sqlite() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO llm_provider_configs VALUES ('claude-code','main','anthropic','https://x','[\"A\"]',1)",
            [],
        )
        .unwrap();
        let rows = read_rows(&conn).unwrap();
        assert_eq!(rows["claude-code"][0].header_names, vec!["A".to_string()]);
    }
}
