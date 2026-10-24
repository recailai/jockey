//! Per-runtime LLM endpoint routing through ACP `providers/*` (unstable). A saved config makes
//! an adapter send its model traffic to the user's own endpoint.
//!
//! Provider settings live in the adapter process, so every fresh connection gets them again:
//! the cold start calls `acp::session::providers::apply_saved_providers` right after
//! `initialize`, before any session call. Header values may carry API keys, so SQLite keeps only
//! header names (`db::llm_providers`); the values live in the OS keychain
//! (`credentials::secrets`).
//!
//! The ACP worker thread has no `AppState` and must never wait on the keychain (a first read can
//! show a prompt), so everything it needs is mirrored in one in-memory [`State`]: configs and
//! secrets are filled at startup (on a helper thread) and updated on every save/clear.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Deserialize;

use crate::acp::protocol as acp;
use crate::db::llm_providers::{self as rows, StoredConfig};
use crate::db::with_db;
use crate::runtime_kind::RuntimeKind;
use crate::types::AppState;

#[derive(Default)]
struct State {
    configs: HashMap<String, Vec<StoredConfig>>,
    /// Keyed by `secret_id`. An `Err` is a keychain read failure, reported per provider.
    secrets: HashMap<String, Result<BTreeMap<String, String>, String>>,
    apply_errors: HashMap<String, String>,
    revisions: HashMap<String, u64>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static REVISION_SEQ: AtomicU64 = AtomicU64::new(1);

/// A poisoned lock only means another thread panicked mid-update of plain maps; the data is
/// still the best view we have, and returning empty would silently drop saved providers.
fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(state().get_or_insert_with(State::default))
}

fn next_revision() -> u64 {
    REVISION_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Changes whenever the runtime's saved config changes. A live connection remembers the value
/// it started with; a mismatch means it was configured with stale settings and is evicted.
pub(crate) fn revision(runtime_key: &str) -> u64 {
    with_state(|s| s.revisions.get(runtime_key).copied().unwrap_or(0))
}

pub(crate) fn secret_id(runtime_key: &str, provider_id: &str) -> String {
    format!("llm-provider:{runtime_key}:{provider_id}")
}

/// Startup: mirror every saved config into memory. Secrets follow via [`preload_secrets`].
pub(crate) fn load(conn: &rusqlite::Connection) -> Result<(), String> {
    let all = rows::read_all(conn)?;
    with_state(|s| s.configs = all);
    Ok(())
}

/// Reads every saved provider's secret off the calling thread's critical path. Until a secret
/// arrives its provider is reported as not yet loaded; revisions are bumped at the end so a
/// connection that started in that window is evicted and re-applied with real values.
pub(crate) fn preload_secrets() {
    let ids: Vec<String> = with_state(|s| {
        s.configs
            .iter()
            .flat_map(|(rt, list)| list.iter().map(move |c| secret_id(rt, &c.provider_id)))
            .collect()
    });
    let spawned = std::thread::Builder::new()
        .name("llm-provider-secrets".into())
        .spawn(move || {
            let loaded: Vec<_> = ids
                .into_iter()
                .map(|id| {
                    let secret = crate::credentials::secrets::load(&id);
                    (id, secret)
                })
                .collect();
            with_state(|s| {
                for (id, secret) in loaded {
                    // A save that raced the preload already holds the newer value.
                    s.secrets.entry(id).or_insert(secret);
                }
                let runtimes: Vec<String> = s.configs.keys().cloned().collect();
                for rt in runtimes {
                    s.revisions.insert(rt, next_revision());
                }
            });
        });
    if let Err(e) = spawned {
        eprintln!("[llm-providers] secret preload failed to start: {e}");
    }
}

pub(crate) fn configs_for(runtime_key: &str) -> Vec<StoredConfig> {
    with_state(|s| s.configs.get(runtime_key).cloned().unwrap_or_default())
}

pub(crate) fn apply_error(runtime_key: &str) -> Option<String> {
    with_state(|s| s.apply_errors.get(runtime_key).cloned())
}

pub(crate) fn parse_protocol(raw: &str) -> acp::LlmProtocol {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .unwrap_or_else(|_| acp::LlmProtocol::Other(raw.to_string()))
}

pub(crate) fn protocol_name(p: &acp::LlmProtocol) -> String {
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
/// provider whose secret is unavailable is an `Err` entry so the caller can report it. Reads
/// memory only, so it is safe on the ACP worker thread.
pub(crate) fn pending_requests(
    runtime_key: &str,
) -> Vec<(String, Result<acp::SetProviderRequest, String>)> {
    with_state(|s| {
        s.configs
            .get(runtime_key)
            .into_iter()
            .flatten()
            .map(|cfg| {
                let req = if cfg.header_names.is_empty() {
                    Ok(build_request(cfg, &BTreeMap::new()))
                } else {
                    match s.secrets.get(&secret_id(runtime_key, &cfg.provider_id)) {
                        Some(Ok(secrets)) => Ok(build_request(cfg, secrets)),
                        Some(Err(e)) => Err(e.clone()),
                        None => Err("credentials are still loading".to_string()),
                    }
                };
                (cfg.provider_id.clone(), req)
            })
            .collect()
    })
}

fn redact(text: &str, secret_values: &[&str]) -> String {
    let mut values: Vec<&str> = secret_values
        .iter()
        .copied()
        .filter(|v| !v.is_empty())
        .collect();
    // Longest first so a value that contains another is not left half-visible.
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values
        .into_iter()
        .fold(text.to_string(), |acc, v| acc.replace(v, "***"))
}

/// Keeps the apply outcome for the settings page; the connection itself never fails on it.
/// Upstream errors can echo request headers, so known secret values are masked first.
pub(crate) fn record_apply(runtime_key: &str, failures: &[String]) {
    with_state(|s| {
        if failures.is_empty() {
            s.apply_errors.remove(runtime_key);
            return;
        }
        let known: Vec<&str> = s
            .configs
            .get(runtime_key)
            .into_iter()
            .flatten()
            .filter_map(|c| s.secrets.get(&secret_id(runtime_key, &c.provider_id)))
            .filter_map(|r| r.as_ref().ok())
            .flat_map(|m| m.values().map(String::as_str))
            .collect();
        let text = redact(&failures.join("; "), &known);
        s.apply_errors.insert(runtime_key.to_string(), text);
    });
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeaderInput {
    pub(crate) name: String,
    /// `None` keeps the value already in the keychain under this name.
    pub(crate) value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderConfigInput {
    pub(crate) provider_id: String,
    pub(crate) api_type: String,
    pub(crate) base_url: String,
    pub(crate) headers: Vec<HeaderInput>,
}

pub(crate) fn runtime_kind(runtime_key: &str) -> Result<RuntimeKind, String> {
    RuntimeKind::from_str(runtime_key).ok_or_else(|| format!("unknown runtime: {runtime_key}"))
}

/// Header values travel with every model request, so a plain-http endpoint is only accepted
/// where the traffic never leaves the machine.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    if let Some(bracketed) = host_port.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or("");
    }
    host_port.split(':').next().unwrap_or("")
}

pub(crate) fn validate(input: &ProviderConfigInput) -> Result<(), String> {
    if input.provider_id.trim().is_empty() {
        return Err("provider id is required".into());
    }
    if input.api_type.trim().is_empty() {
        return Err("protocol is required".into());
    }
    let url = input.base_url.trim();
    if url.starts_with("http://") {
        let host = host_of(url).to_ascii_lowercase();
        if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
            return Err("http:// is only allowed for localhost; use https://".into());
        }
    } else if !url.starts_with("https://") {
        return Err("base URL must start with http:// or https://".into());
    }
    if host_of(url).is_empty() {
        return Err("base URL has no host".into());
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

/// Publishes a committed change: reload only this runtime's rows, replace the provider's
/// secret, drop the stale apply error and bump the revision so live connections are evicted.
fn publish(
    runtime_key: &str,
    configs: Vec<StoredConfig>,
    id: String,
    secret: Option<BTreeMap<String, String>>,
) {
    with_state(|s| {
        if configs.is_empty() {
            s.configs.remove(runtime_key);
        } else {
            s.configs.insert(runtime_key.to_string(), configs);
        }
        match secret {
            Some(v) => s.secrets.insert(id, Ok(v)),
            None => s.secrets.remove(&id),
        };
        s.apply_errors.remove(runtime_key);
        s.revisions.insert(runtime_key.to_string(), next_revision());
    });
}

fn known_secret(id: &str) -> Result<BTreeMap<String, String>, String> {
    if let Some(Ok(v)) = with_state(|s| s.secrets.get(id).cloned()) {
        return Ok(v);
    }
    crate::credentials::secrets::load(id)
}

/// Blocking (keychain). The DB row is written first and the keychain second; if the keychain
/// write fails the row is restored, so names and secrets never diverge.
pub(crate) fn save(
    state: &AppState,
    runtime_key: &str,
    config: &ProviderConfigInput,
) -> Result<(), String> {
    runtime_kind(runtime_key)?;
    validate(config)?;
    let provider_id = config.provider_id.trim().to_string();
    let id = secret_id(runtime_key, &provider_id);
    let merged = merge_secrets(&known_secret(&id)?, &config.headers);
    let next = StoredConfig {
        provider_id: provider_id.clone(),
        api_type: config.api_type.trim().to_string(),
        base_url: config.base_url.trim().to_string(),
        header_names: config
            .headers
            .iter()
            .map(|h| h.name.trim().to_string())
            .collect(),
    };
    let previous = with_db(state, |c| rows::read_one(c, runtime_key, &provider_id))?;
    with_db(state, |c| rows::upsert(c, runtime_key, &next))?;
    if let Err(e) = crate::credentials::secrets::save(&id, &merged) {
        let _ = with_db(state, |c| match &previous {
            Some(prev) => rows::upsert(c, runtime_key, prev),
            None => rows::delete(c, runtime_key, &provider_id),
        });
        return Err(e);
    }
    let configs = with_db(state, |c| rows::read_runtime(c, runtime_key))?;
    publish(runtime_key, configs, id, Some(merged));
    Ok(())
}

/// Blocking (keychain). Mirror of [`save`]: row first, keychain second, row restored on failure.
pub(crate) fn clear(state: &AppState, runtime_key: &str, provider_id: &str) -> Result<(), String> {
    let id = secret_id(runtime_key, provider_id);
    let previous = with_db(state, |c| rows::read_one(c, runtime_key, provider_id))?;
    with_db(state, |c| rows::delete(c, runtime_key, provider_id))?;
    if let Err(e) = crate::credentials::secrets::delete(&id) {
        if let Some(prev) = &previous {
            let _ = with_db(state, |c| rows::upsert(c, runtime_key, prev));
        }
        return Err(e);
    }
    let configs = with_db(state, |c| rows::read_runtime(c, runtime_key))?;
    publish(runtime_key, configs, id, None);
    Ok(())
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
    fn plain_http_is_only_for_loopback() {
        let with_url = |u: &str| {
            let mut i = input(vec![]);
            i.base_url = u.into();
            validate(&i)
        };
        assert!(with_url("http://localhost:11434/v1").is_ok());
        assert!(with_url("http://127.0.0.1:8080").is_ok());
        assert!(with_url("http://[::1]:8080/x").is_ok());
        assert!(with_url("http://llm.example.com").is_err());
        assert!(with_url("http://localhost@evil.com/").is_err());
        assert!(with_url("https://llm.example.com").is_ok());
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
    fn redact_masks_known_values_longest_first() {
        let out = redact(
            "bad key Bearer sk-abc and sk-abc",
            &["sk-abc", "Bearer sk-abc", ""],
        );
        assert_eq!(out, "bad key *** and ***");
    }

    #[test]
    fn apply_errors_are_redacted_and_state_survives_poisoning() {
        let rt = "test-runtime-redact";
        let cfg = StoredConfig {
            provider_id: "p".into(),
            api_type: "openai".into(),
            base_url: "https://x".into(),
            header_names: vec!["Authorization".into()],
        };
        publish(
            rt,
            vec![cfg],
            secret_id(rt, "p"),
            Some(BTreeMap::from([(
                "Authorization".into(),
                "topsecret".into(),
            )])),
        );
        let rev = revision(rt);
        assert!(rev > 0);
        record_apply(rt, &["p: rejected header topsecret".to_string()]);
        let err = apply_error(rt).unwrap();
        assert!(err.contains("***") && !err.contains("topsecret"));
        record_apply(rt, &[]);
        assert!(apply_error(rt).is_none());
        let pending = pending_requests(rt);
        assert!(pending[0].1.is_ok());
        publish(rt, vec![], secret_id(rt, "p"), None);
        assert!(revision(rt) > rev);
        assert!(pending_requests(rt).is_empty());
    }
}
