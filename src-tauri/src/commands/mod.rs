pub(crate) mod completion;
mod role_templates;

use crate::assistant::normalize_runtime_key;
use crate::db::app_session_role::{
    load_app_session_role_state, save_app_session_role_model_override,
};
use crate::db::context::*;
use crate::db::role::resolve_role_runtime;
use crate::db::session_context::{
    app_session_role_scope, app_session_scope, list_shared_context_prefix_internal,
};
use crate::db::{get_state, with_db};
use crate::types::*;
use crate::{build_unionai_tool_prompt, now_ms, resolve_chat_cwd};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

pub(crate) fn split_tokens(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

pub(crate) fn enrich_command_message(base: &str, payload: &Value) -> String {
    let Value::Object(map) = payload else {
        return base.to_string();
    };
    if map.is_empty() {
        return base.to_string();
    }
    if let Some(Value::String(help)) = map.get("help") {
        return format!("{base}\n{help}");
    }
    let detail = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    format!("{base}\n{detail}")
}

fn required_app_session_id(app_session_id: Option<&str>) -> Result<&str, String> {
    app_session_id
        .filter(|sid| !sid.trim().is_empty())
        .ok_or_else(|| "app session id required".to_string())
}

#[tauri::command]
pub(crate) async fn apply_chat_command(
    app: AppHandle,
    state: State<'_, AppState>,
    input: String,
    runtime_kind: Option<String>,
    app_session_id: Option<String>,
) -> Result<ChatCommandResult, String> {
    let trimmed = input.trim();
    if !trimmed.starts_with("/app_") {
        return Err("app commands must start with /app".to_string());
    }

    let tokens = split_tokens(trimmed);
    if tokens.is_empty() {
        return Err("empty command".to_string());
    }

    let mut result = ChatCommandResult {
        ok: true,
        message: "ok".to_string(),
        runtime_kind: runtime_kind.clone(),
        session_id: None,
        payload: json!({}),
    };

    let app_session_id_ref = app_session_id.as_deref().filter(|s| !s.trim().is_empty());
    let assistant_scope =
        app_session_id_ref.map(|sid| app_session_role_scope(sid, "UnionAIAssistant"));

    if role_templates::handle_role_template_command(state.clone(), &tokens, &mut result)? {
        result.message = enrich_command_message(&result.message, &result.payload);
        app.emit("command/applied", result.clone())
            .map_err(|e| e.to_string())?;
        return Ok(result);
    }

    match tokens.as_slice() {
        ["/app_help"] => {
            result.message = "command list".to_string();
            result.payload = json!({ "help": build_unionai_tool_prompt() });
        }
        ["/app_cd"] => {
            let cwd = if let Some(ref sid) = app_session_id {
                crate::db::app_session::get_app_session_cwd(get_state(&state), sid)
                    .unwrap_or_else(|| resolve_chat_cwd())
            } else {
                resolve_chat_cwd()
            };
            result.message = format!("cwd: {}", cwd);
            result.payload = json!({ "cwd": cwd });
        }
        ["/app_cd", path] => {
            let resolved = crate::abs_cwd(path);
            if !std::path::Path::new(&resolved).is_dir() {
                return Err(format!("not a directory: {}", resolved));
            }
            if let Some(ref sid) = app_session_id {
                crate::db::app_session::set_app_session_cwd(get_state(&state), sid, &resolved)?;
            }
            result.message = format!("cwd changed: {}", resolved);
            result.payload = json!({ "cwd": resolved });
        }
        ["/app_assistant", "list"] => {
            let assistants = crate::assistant::assistant_catalog();
            result.message = "assistant list".to_string();
            result.payload = json!({ "assistants": assistants });
        }
        ["/app_assistant", "select", runtime] => match normalize_runtime_key(runtime) {
            Some(normalized) => {
                result.runtime_kind = Some(normalized.to_string());
                result.message = format!("assistant selected: {}", normalized);
                result.payload = json!({ "assistant": normalized });
            }
            None => {
                result.ok = false;
                result.message = format!("unsupported assistant: {}", runtime);
                result.payload = json!({ "assistant": runtime });
            }
        },
        ["/app_model", "list"] => {
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let models = list_models_for_runtime(get_state(&state), &runtime)?;
            let selected = app_session_id_ref
                .and_then(|sid| load_app_session_role_state(get_state(&state), sid, "UnionAIAssistant").ok().flatten())
                .and_then(|row| row.model_override)
                .map(|value| ContextEntry {
                    scope: app_session_role_scope(app_session_id_ref.unwrap(), "UnionAIAssistant"),
                    key: "model".to_string(),
                    value,
                    updated_at: now_ms(),
                });
            result.message = format!("{} models for runtime {}", models.len(), runtime);
            result.payload = json!({
                "runtime": runtime,
                "models": models,
                "selected": selected
            });
        }
        ["/app_model", "add", model] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "model", model)?;
            result.message = format!("model added: {}", name);
            result.payload = json!({ "model": name });
        }
        ["/app_model", "remove", model] => {
            let removed = remove_dynamic_catalog_item(get_state(&state), "model", model)?;
            result.ok = removed;
            result.message = if removed {
                format!("model removed: {}", model)
            } else {
                format!("model not found: {}", model)
            };
            result.payload = json!({ "model": model, "removed": removed });
        }
        ["/app_model", "select", "role", role_name, model] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = resolve_role_runtime(state.clone(), role_name)?;
            let _ = upsert_dynamic_catalog_item(get_state(&state), "model", &selected_model)?;
            save_app_session_role_model_override(
                get_state(&state),
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
        }
        ["/app_model", "select", model] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            let _ = upsert_dynamic_catalog_item(get_state(&state), "model", &selected_model)?;
            save_app_session_role_model_override(
                get_state(&state),
                sid,
                "UnionAIAssistant",
                &runtime,
                Some(&selected_model),
            )?;
            let entry = ContextEntry {
                scope: app_session_role_scope(sid, "UnionAIAssistant"),
                key: "model".to_string(),
                value: selected_model,
                updated_at: now_ms(),
            };
            result.message = format!("model selected: {}", entry.value);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "get", "role", role_name] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let entry = load_app_session_role_state(get_state(&state), sid, role_name)?
                .and_then(|row| {
                    row.model_override.map(|value| ContextEntry {
                        scope: app_session_role_scope(sid, role_name),
                        key: "model".to_string(),
                        value,
                        updated_at: now_ms(),
                    })
                });
            let runtime = resolve_role_runtime(state.clone(), role_name)?;
            result.message = format!("model fetched for role {}", role_name);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "get"] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let entry = load_app_session_role_state(get_state(&state), sid, "UnionAIAssistant")?
                .and_then(|row| {
                    row.model_override.map(|value| ContextEntry {
                        scope: app_session_role_scope(sid, "UnionAIAssistant"),
                        key: "model".to_string(),
                        value,
                        updated_at: now_ms(),
                    })
                });
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            result.message = "model fetched".to_string();
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "clear", "role", role_name] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let runtime = resolve_role_runtime(state.clone(), role_name)?;
            save_app_session_role_model_override(get_state(&state), sid, role_name, &runtime, None)?;
            result.message = format!("model cleared for role {}", role_name);
            result.payload = json!({ "role": role_name });
        }
        ["/app_model", "clear"] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let runtime = resolve_model_runtime(result.runtime_kind.as_deref());
            save_app_session_role_model_override(
                get_state(&state),
                sid,
                "UnionAIAssistant",
                &runtime,
                None,
            )?;
            result.message = "model cleared".to_string();
            result.payload = json!({});
        }
        ["/app_mcp", "list"] => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let catalog = list_dynamic_catalog(get_state(&state), "mcp")?;
            let enabled = list_enabled_feature_flags(get_state(&state), &scope, "mcp:");
            result.message = format!("{} MCP servers enabled", enabled.len());
            result.payload = json!({ "catalog": catalog, "enabled": enabled });
        }
        ["/app_mcp", "add", server] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "mcp", server)?;
            result.message = format!("mcp server added: {}", name);
            result.payload = json!({ "server": name });
        }
        ["/app_mcp", "remove", server] => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            let removed = remove_dynamic_catalog_item(get_state(&state), "mcp", &name)?;
            if removed {
                clear_shared_context_internal(
                    get_state(&state),
                    &scope,
                    &format!("mcp:{name}"),
                )?;
            }
            result.ok = removed;
            result.message = if removed {
                format!("mcp server removed: {}", name)
            } else {
                format!("mcp server not found: {}", name)
            };
            result.payload = json!({ "server": name, "removed": removed });
        }
        ["/app_mcp", mode, server] if *mode == "enable" || *mode == "disable" => {
            let scope = assistant_scope
                .clone()
                .ok_or_else(|| "app session id required".to_string())?;
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            if !dynamic_catalog_contains(get_state(&state), "mcp", &name)? {
                let supported = list_dynamic_catalog(get_state(&state), "mcp")?;
                result.ok = false;
                result.message = format!("unsupported mcp server: {}", server);
                result.payload = json!({ "supported": supported });
            } else {
                let value = if *mode == "enable" {
                    "enabled"
                } else {
                    "disabled"
                };
                let entry = set_shared_context_internal(
                    get_state(&state),
                    &scope,
                    &format!("mcp:{name}"),
                    value,
                )?;
                result.message = format!("mcp {}: {}", mode, name);
                result.payload = json!({ "entry": entry });
            }
        }
        ["/app_context", "list"] | ["/app_context", "list", ..] => {
            let sid = required_app_session_id(app_session_id_ref)?;
            let prefix = app_session_scope(sid);
            let scoped_prefix = format!("{prefix}:");
            let scope = tokens.get(2).copied().unwrap_or("");
            let entries = if scope.is_empty() {
                list_shared_context_prefix_internal(get_state(&state), &prefix)?
                    .into_iter()
                    .filter(|entry| {
                        entry.scope == prefix || entry.scope.starts_with(&scoped_prefix)
                    })
                    .collect()
            } else {
                if !(scope == prefix || scope.starts_with(&scoped_prefix)) {
                    return Err(format!("scope must stay within {}", prefix));
                }
                list_shared_context_internal(get_state(&state), scope)?
            };
            result.message = format!(
                "{} context entries{}",
                entries.len(),
                if scope.is_empty() {
                    String::new()
                } else {
                    format!(" (scope: {})", scope)
                }
            );
            result.payload = json!({ "entries": entries });
        }
        ["/app_session", "list"] => {
            let sessions = with_db(get_state(&state), |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, workflow_id, status, initial_prompt, created_at, updated_at
                         FROM sessions ORDER BY created_at DESC LIMIT 50",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(Session {
                            id: row.get(0)?,
                            workflow_id: row.get(1)?,
                            status: row.get(2)?,
                            initial_prompt: row.get(3)?,
                            created_at: row.get(4)?,
                            updated_at: row.get(5)?,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                let mut items = Vec::new();
                for row in rows {
                    items.push(row.map_err(|e| e.to_string())?);
                }
                Ok(items)
            })?;
            result.message = format!("{} sessions", sessions.len());
            result.payload = json!({ "sessions": sessions });
        }
        ["/app_workflow", "list"] => {
            let workflows = with_db(get_state(&state), |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, name, steps_json, created_at, updated_at
                         FROM workflows ORDER BY updated_at DESC",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |row| {
                        let steps_json: String = row.get(2)?;
                        let steps =
                            serde_json::from_str::<Vec<String>>(&steps_json).unwrap_or_default();
                        Ok(Workflow {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            steps,
                            created_at: row.get(3)?,
                            updated_at: row.get(4)?,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                let mut items = Vec::new();
                for row in rows {
                    items.push(row.map_err(|e| e.to_string())?);
                }
                Ok(items)
            })?;
            result.message = format!("{} workflows", workflows.len());
            result.payload = json!({ "workflows": workflows });
        }
        ["/app_team", ..] => {
            result.ok = false;
            result.message = "workspace commands are managed automatically.".to_string();
            result.payload = json!({});
        }
        _ => {
            result.ok = false;
            result.message = "unsupported command".to_string();
        }
    }
    result.message = enrich_command_message(&result.message, &result.payload);

    app.emit("command/applied", result.clone())
        .map_err(|e| e.to_string())?;
    Ok(result)
}
