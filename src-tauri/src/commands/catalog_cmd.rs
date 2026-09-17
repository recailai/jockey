use crate::db::app_session_role::{
    load_app_session_role_state, save_app_session_role_model_override,
};
use crate::db::context::{
    clear_shared_context_internal, dynamic_catalog_contains, list_dynamic_catalog,
    list_enabled_feature_flags, list_models_for_runtime, remove_dynamic_catalog_item,
    resolve_model_runtime, sanitize_dynamic_item_name, set_shared_context_internal,
    upsert_dynamic_catalog_item,
};
use crate::db::role::load_role_runtime_kind;
use crate::db::session_context::app_session_role_scope;
use crate::now_ms;
use crate::types::{AppState, ChatCommandResult, ContextEntry};
use rusqlite::OptionalExtension;
use serde_json::json;

fn required_app_session_id(app_session_id: Option<&str>) -> Result<&str, String> {
    app_session_id
        .filter(|sid| !sid.trim().is_empty())
        .ok_or_else(|| "app session id required".to_string())
}

fn is_claude_family_alias(model_lc: &str) -> bool {
    matches!(model_lc, "sonnet" | "haiku" | "opus" | "fable") || model_lc.starts_with("claude-")
}

/// Rank a Claude model id so a bare family alias resolves to the newest release.
/// Lexical ordering is wrong here (`claude-opus-4-8` sorts above `claude-opus-5`), and the
/// `[1m]` variants must lose to their base model — asking for "opus" should not silently
/// opt into a 1M-context session.
fn claude_model_rank(model_lc: &str) -> (u32, u32, u32) {
    let base = model_lc.trim_end_matches("[1m]");
    let base_preferred = u32::from(base.len() == model_lc.len());
    let mut parts = base
        .rsplit('-')
        .map_while(|part| part.parse::<u32>().ok())
        .collect::<Vec<_>>();
    parts.reverse();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    (major, minor, base_preferred)
}

fn claude_alias_candidates(models: &[String], alias_lc: &str) -> Vec<String> {
    models
        .iter()
        .filter(|m| {
            let m_lc = m.to_ascii_lowercase();
            m_lc.starts_with("claude-")
                && (m_lc.contains(&format!("-{}-", alias_lc))
                    || m_lc.ends_with(&format!("-{}", alias_lc)))
        })
        .cloned()
        .collect()
}

fn normalize_selected_model_for_runtime(
    state: &AppState,
    runtime: &str,
    selected_model: &str,
) -> Result<String, String> {
    let selected = selected_model.trim();
    if selected.is_empty() {
        return Ok(String::new());
    }
    let selected_lc = selected.to_ascii_lowercase();
    let models = list_models_for_runtime(state, runtime).unwrap_or_default();
    // Compatibility is decided by the runtime's own catalog, not by a runtime-key allowlist:
    // both Claude runtimes serve Claude models, and Antigravity proxies some of them too.
    let runtime_serves_claude = runtime.contains("claude")
        || models
            .iter()
            .any(|m| m.to_ascii_lowercase().starts_with("claude-"));
    if is_claude_family_alias(&selected_lc) && !runtime_serves_claude {
        return Err(format!(
            "model '{}' is not compatible with runtime {}",
            selected, runtime
        ));
    }
    if models.is_empty() {
        return Ok(selected.to_string());
    }
    if let Some(exact) = models
        .iter()
        .find(|m| m.trim().eq_ignore_ascii_case(selected))
    {
        return Ok(exact.clone());
    }
    if matches!(selected_lc.as_str(), "sonnet" | "haiku" | "opus" | "fable") {
        let candidates = claude_alias_candidates(&models, &selected_lc);
        if let Some(best) = candidates
            .iter()
            .max_by_key(|m| claude_model_rank(&m.to_ascii_lowercase()))
        {
            return Ok(best.clone());
        }
    }
    Ok(selected.to_string())
}

pub(crate) fn handle_catalog_command(
    tokens: &[&str],
    state: &AppState,
    app_session_id_ref: Option<&str>,
    result: &mut ChatCommandResult,
) -> Result<bool, String> {
    let active_role = app_session_id_ref
        .and_then(|sid| {
            crate::db::with_db(state, |conn| {
                conn.query_row(
                    "SELECT active_role FROM app_sessions WHERE id = ?1",
                    rusqlite::params![sid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())
            })
            .ok()
            .flatten()
        })
        .unwrap_or_else(|| "Developer".to_string());
    let assistant_scope = app_session_id_ref.map(|sid| app_session_role_scope(sid, &active_role));
    match tokens {
        ["/app_model", "list"] => {
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let models = list_models_for_runtime(state, &runtime)?;
            let selected = app_session_id_ref
                .and_then(|sid| {
                    load_app_session_role_state(state, sid, &active_role)
                        .ok()
                        .flatten()
                })
                .and_then(|row| row.model_override)
                .and_then(|value| {
                    app_session_id_ref.map(|sid| ContextEntry {
                        scope: app_session_role_scope(sid, &active_role),
                        key: "model".to_string(),
                        value,
                        updated_at: now_ms(),
                    })
                });
            result.message = format!("{} models for runtime {}", models.len(), runtime);
            result.payload = json!({
                "runtime": runtime,
                "models": models,
                "selected": selected
            });
            Ok(true)
        }
        ["/app_model", "add", model] => {
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let name = upsert_dynamic_catalog_item(state, "model", &runtime, model)?;
            result.message = format!("model added: {}", name);
            result.payload = json!({ "model": name });
            Ok(true)
        }
        ["/app_model", "remove", model] => {
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let removed = remove_dynamic_catalog_item(state, "model", &runtime, model)?;
            result.ok = removed;
            result.message = if removed {
                format!("model removed: {}", model)
            } else {
                format!("model not found: {}", model)
            };
            result.payload = json!({ "model": model, "removed": removed });
            Ok(true)
        }
        ["/app_model", "select", "role", role_name, model] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = load_role_runtime_kind(state, role_name)?;
            let selected_model =
                normalize_selected_model_for_runtime(state, &runtime, &selected_model)?;
            let _ = upsert_dynamic_catalog_item(state, "model", &runtime, &selected_model)?;
            save_app_session_role_model_override(
                state,
                sid,
                role_name,
                &runtime,
                Some(&selected_model),
            )?;
            let entry = ContextEntry {
                scope: app_session_role_scope(sid, role_name),
                key: "model".to_string(),
                value: selected_model,
                updated_at: now_ms(),
            };
            result.message = format!("model selected for role {}: {}", role_name, entry.value);
            result.payload = json!({ "entry": entry, "runtime": runtime });
            Ok(true)
        }
        ["/app_model", "select", model] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let selected_model =
                normalize_selected_model_for_runtime(state, &runtime, &selected_model)?;
            let _ = upsert_dynamic_catalog_item(state, "model", &runtime, &selected_model)?;
            save_app_session_role_model_override(
                state,
                sid,
                &active_role,
                &runtime,
                Some(&selected_model),
            )?;
            let entry = ContextEntry {
                scope: app_session_role_scope(sid, &active_role),
                key: "model".to_string(),
                value: selected_model,
                updated_at: now_ms(),
            };
            result.message = format!("model selected: {}", entry.value);
            result.payload = json!({ "entry": entry, "runtime": runtime });
            Ok(true)
        }
        ["/app_model", "get", "role", role_name] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let entry = load_app_session_role_state(state, sid, role_name)?.and_then(|row| {
                row.model_override.map(|value| ContextEntry {
                    scope: app_session_role_scope(sid, role_name),
                    key: "model".to_string(),
                    value,
                    updated_at: now_ms(),
                })
            });
            let runtime = load_role_runtime_kind(state, role_name)?;
            result.message = format!("model fetched for role {}", role_name);
            result.payload = json!({ "entry": entry, "runtime": runtime });
            Ok(true)
        }
        ["/app_model", "get"] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let entry = load_app_session_role_state(state, sid, &active_role)?.and_then(|row| {
                row.model_override.map(|value| ContextEntry {
                    scope: app_session_role_scope(sid, &active_role),
                    key: "model".to_string(),
                    value,
                    updated_at: now_ms(),
                })
            });
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            result.message = "model fetched".to_string();
            result.payload = json!({ "entry": entry, "runtime": runtime });
            Ok(true)
        }
        ["/app_model", "clear", "role", role_name] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let runtime = load_role_runtime_kind(state, role_name)?;
            save_app_session_role_model_override(state, sid, role_name, &runtime, None)?;
            result.message = format!("model cleared for role {}", role_name);
            result.payload = json!({ "role": role_name });
            Ok(true)
        }
        ["/app_model", "clear"] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            save_app_session_role_model_override(state, sid, &active_role, &runtime, None)?;
            result.message = "model cleared".to_string();
            result.payload = json!({});
            Ok(true)
        }
        ["/app_mcp", "list"] => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let catalog = list_dynamic_catalog(state, "mcp", "")?;
            let enabled = list_enabled_feature_flags(state, &scope, "mcp:");
            result.message = format!("{} MCP servers enabled", enabled.len());
            result.payload = json!({ "catalog": catalog, "enabled": enabled });
            Ok(true)
        }
        ["/app_mcp", "add", server] => {
            let name = upsert_dynamic_catalog_item(state, "mcp", "", server)?;
            result.message = format!("mcp server added: {}", name);
            result.payload = json!({ "server": name });
            Ok(true)
        }
        ["/app_mcp", "remove", server] => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            let removed = remove_dynamic_catalog_item(state, "mcp", "", &name)?;
            if removed {
                clear_shared_context_internal(state, &scope, &format!("mcp:{name}"))?;
            }
            result.ok = removed;
            result.message = if removed {
                format!("mcp server removed: {}", name)
            } else {
                format!("mcp server not found: {}", name)
            };
            result.payload = json!({ "server": name, "removed": removed });
            Ok(true)
        }
        ["/app_mcp", mode, server] if *mode == "enable" || *mode == "disable" => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            if !dynamic_catalog_contains(state, "mcp", "", &name)? {
                let supported = list_dynamic_catalog(state, "mcp", "")?;
                result.ok = false;
                result.message = format!("unsupported mcp server: {}", server);
                result.payload = json!({ "supported": supported });
            } else {
                let value = if *mode == "enable" {
                    "enabled"
                } else {
                    "disabled"
                };
                let entry =
                    set_shared_context_internal(state, &scope, &format!("mcp:{name}"), value)?;
                result.message = format!("mcp {}: {}", mode, name);
                result.payload = json!({ "entry": entry });
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
