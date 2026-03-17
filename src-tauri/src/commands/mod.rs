pub(crate) mod completion;

use crate::assistant::normalize_runtime_key;
use crate::db::context::*;
use crate::db::role::*;
use crate::db::{ensure_default_team_id, get_state, team_exists, with_db};
use crate::types::*;
use crate::{acp, build_unionai_tool_prompt, now_ms, resolve_chat_cwd};
use rusqlite::params;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

pub(crate) fn split_tokens(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

pub(crate) fn ensure_team_selected(
    state: State<'_, AppState>,
    selected_team_id: Option<String>,
) -> Result<String, String> {
    if let Some(team_id) = selected_team_id {
        if team_exists(get_state(&state), &team_id)? {
            return Ok(team_id);
        }
    }
    ensure_default_team_id(get_state(&state))
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

#[tauri::command]
pub(crate) async fn apply_chat_command(
    app: AppHandle,
    state: State<'_, AppState>,
    input: String,
    selected_team_id: Option<String>,
    selected_assistant: Option<String>,
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
        selected_team_id: None,
        selected_assistant: selected_assistant.clone(),
        session_id: None,
        payload: json!({}),
    };
    let active_team_id = ensure_team_selected(state.clone(), selected_team_id.clone())?;
    result.selected_team_id = Some(active_team_id.clone());

    match tokens.as_slice() {
        ["/app_help"] => {
            result.message = "command list".to_string();
            result.payload = json!({ "help": build_unionai_tool_prompt() });
        }
        ["/app_assistant", "list"] => {
            let assistants = crate::assistant::assistant_catalog();
            result.message = "assistant list".to_string();
            result.payload = json!({ "assistants": assistants });
        }
        ["/app_assistant", "select", runtime] => match normalize_runtime_key(runtime) {
            Some(normalized) => {
                result.selected_assistant = Some(normalized.to_string());
                result.message = format!("assistant selected: {}", normalized);
                result.payload = json!({ "assistant": normalized });
                let runtime_key = normalized.to_string();
                let prewarm_cwd = resolve_chat_cwd(get_state(&state), Some(&active_team_id));
                tauri::async_runtime::spawn(async move {
                    acp::prewarm(&runtime_key, &prewarm_cwd).await;
                });
            }
            None => {
                result.ok = false;
                result.message = format!("unsupported assistant: {}", runtime);
                result.payload = json!({ "assistant": runtime });
            }
        },
        ["/app_model", "list"] => {
            let runtime = resolve_model_runtime(result.selected_assistant.as_deref());
            let models = list_models_for_runtime(get_state(&state), &runtime)?;
            let selected = get_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                "model".to_string(),
            )?;
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
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            let _ = upsert_dynamic_catalog_item(get_state(&state), "model", &selected_model)?;
            let entry = set_shared_context(
                state.clone(),
                context_scope_for_role(role_name),
                "model".to_string(),
                selected_model,
            )?;
            result.message = format!("model selected for role {}: {}", role_name, entry.value);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "select", model] => {
            let selected_model = sanitize_dynamic_item_name(model)
                .ok_or_else(|| format!("invalid model name: {}", model))?;
            let runtime = resolve_model_runtime(result.selected_assistant.as_deref());
            let _ = upsert_dynamic_catalog_item(get_state(&state), "model", &selected_model)?;
            let entry = set_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                "model".to_string(),
                selected_model,
            )?;
            result.message = format!("model selected: {}", entry.value);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "get", "role", role_name] => {
            let entry = get_shared_context(
                state.clone(),
                context_scope_for_role(role_name),
                "model".to_string(),
            )?;
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            result.message = format!("model fetched for role {}", role_name);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "get"] => {
            let entry = get_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                "model".to_string(),
            )?;
            let runtime = resolve_model_runtime(result.selected_assistant.as_deref());
            result.message = "model fetched".to_string();
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/app_model", "clear", "role", role_name] => {
            let scope = context_scope_for_role(role_name);
            clear_shared_context_internal(get_state(&state), &scope, "model")?;
            result.message = format!("model cleared for role {}", role_name);
            result.payload = json!({ "role": role_name });
        }
        ["/app_model", "clear"] => {
            clear_shared_context_internal(get_state(&state), "assistant:main", "model")?;
            result.message = "model cleared".to_string();
            result.payload = json!({});
        }
        ["/app_mcp", "list"] => {
            let catalog = list_dynamic_catalog(get_state(&state), "mcp")?;
            let enabled = list_enabled_feature_flags(get_state(&state), "mcp:");
            result.message = format!("{} MCP servers enabled", enabled.len());
            result.payload = json!({ "catalog": catalog, "enabled": enabled });
        }
        ["/app_mcp", "add", server] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "mcp", server)?;
            result.message = format!("mcp server added: {}", name);
            result.payload = json!({ "server": name });
        }
        ["/app_mcp", "remove", server] => {
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            let removed = remove_dynamic_catalog_item(get_state(&state), "mcp", &name)?;
            if removed {
                clear_shared_context_internal(
                    get_state(&state),
                    "assistant:main",
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
                let entry = set_shared_context(
                    state.clone(),
                    "assistant:main".to_string(),
                    format!("mcp:{name}"),
                    value.to_string(),
                )?;
                result.message = format!("mcp {}: {}", mode, name);
                result.payload = json!({ "entry": entry });
            }
        }
        ["/app_team", ..] => {
            result.ok = false;
            result.message = "workspace commands are managed automatically.".to_string();
            result.payload = json!({});
        }
        ["/app_role", "list"] => {
            let roles = list_roles_for_team(get_state(&state), &active_team_id)?;
            result.message = format!("{} roles", roles.len());
            result.payload = json!({ "roles": roles });
        }
        ["/app_role", "bind", role_name, runtime_kind, prompt @ ..] => {
            let normalized_runtime = normalize_runtime_key(runtime_kind)
                .map(|v| v.to_string())
                .unwrap_or_else(|| (*runtime_kind).to_string());
            let role = upsert_role(
                state.clone(),
                active_team_id.clone(),
                (*role_name).to_string(),
                normalized_runtime.clone(),
                if prompt.is_empty() {
                    "default-system-prompt".to_string()
                } else {
                    prompt.join(" ")
                },
                None,
                None,
                None,
                None,
                None,
            )?;
            result.message = format!("role bound: {}", role.role_name);
            result.payload = json!({ "role": role });
            let runtime_for_warmup = normalized_runtime.clone();
            let role_for_warmup = (*role_name).to_string();
            let prewarm_cwd = resolve_chat_cwd(get_state(&state), Some(&active_team_id));
            tauri::async_runtime::spawn(async move {
                acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &prewarm_cwd, None).await;
            });
        }
        ["/app_role", "prompt", role_name, prompt @ ..] => {
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            let role = upsert_role(
                state.clone(),
                active_team_id.clone(),
                (*role_name).to_string(),
                runtime,
                prompt.join(" "),
                None,
                None,
                None,
                None,
                None,
            )?;
            result.message = format!("role prompt updated: {}", role.role_name);
            result.payload = json!({ "role": role });
        }
        ["/app_role", "delete", role_name] => {
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, role_name],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role deleted: {}", role_name);
            result.payload = json!({ "roleName": role_name });
        }
        ["/app_role", "edit", role_name, "model", value @ ..] => {
            let model_value = if value.is_empty() {
                None
            } else {
                Some(value.join(" "))
            };
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE roles SET model = ?1, updated_at = ?2 WHERE team_id = ?3 AND role_name = ?4",
                    params![model_value, now_ms(), &active_team_id, role_name],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role {} model updated", role_name);
            result.payload = json!({ "role": role_name, "model": model_value });
        }
        ["/app_role", "edit", role_name, "mode", value @ ..] => {
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            let available_modes = acp::list_discovered_modes(&runtime);
            if value.is_empty() {
                result.message = format!(
                    "role {} available modes for {}: {}",
                    role_name,
                    runtime,
                    if available_modes.is_empty() {
                        "<unknown yet, run the role once to discover>".to_string()
                    } else {
                        available_modes.join(", ")
                    }
                );
                result.payload = json!({ "role": role_name, "runtime": runtime, "availableModes": available_modes });
            } else {
                let requested_mode = value.join(" ");
                let mode_value = if requested_mode.eq_ignore_ascii_case("none")
                    || requested_mode.eq_ignore_ascii_case("clear")
                {
                    None
                } else {
                    Some(requested_mode.clone())
                };
                if let Some(ref selected) = mode_value {
                    if !available_modes.is_empty()
                        && !available_modes
                            .iter()
                            .any(|m| m.eq_ignore_ascii_case(selected))
                    {
                        result.ok = false;
                        result.message =
                            format!("unsupported mode '{}' for runtime {}", selected, runtime);
                        result.payload = json!({ "role": role_name, "runtime": runtime, "availableModes": available_modes });
                    }
                }
                if result.ok {
                    with_db(get_state(&state), |conn| {
                        conn.execute(
                            "UPDATE roles SET mode = ?1, updated_at = ?2 WHERE team_id = ?3 AND role_name = ?4",
                            params![mode_value, now_ms(), &active_team_id, role_name],
                        ).map_err(|e| e.to_string())?;
                        Ok(())
                    })?;
                    result.message = format!("role {} mode updated", role_name);
                    result.payload = json!({ "role": role_name, "mode": mode_value });
                }
            }
        }
        ["/app_role", "edit", role_name, "auto-approve", value] => {
            let auto = *value == "true" || *value == "1" || *value == "yes";
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE roles SET auto_approve = ?1, updated_at = ?2 WHERE team_id = ?3 AND role_name = ?4",
                    params![auto, now_ms(), &active_team_id, role_name],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role {} auto-approve: {}", role_name, auto);
            result.payload = json!({ "role": role_name, "autoApprove": auto });
        }
        ["/app_role", "edit", role_name, "mcp-add", server_json @ ..] => {
            let json_str = server_json.join(" ");
            let current = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT mcp_servers_json FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, role_name],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())
            })?;
            let mut servers: Vec<Value> = serde_json::from_str(&current).unwrap_or_default();
            let new_server: Value = serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
            servers.push(new_server);
            let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE roles SET mcp_servers_json = ?1, updated_at = ?2 WHERE team_id = ?3 AND role_name = ?4",
                    params![updated, now_ms(), &active_team_id, role_name],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role {} mcp server added", role_name);
            result.payload = json!({ "role": role_name, "mcpServers": servers });
        }
        ["/app_role", "edit", role_name, "mcp-remove", name] => {
            let current = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT mcp_servers_json FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, role_name],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())
            })?;
            let mut servers: Vec<Value> = serde_json::from_str(&current).unwrap_or_default();
            servers.retain(|s| s.get("name").and_then(|n| n.as_str()) != Some(name));
            let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE roles SET mcp_servers_json = ?1, updated_at = ?2 WHERE team_id = ?3 AND role_name = ?4",
                    params![updated, now_ms(), &active_team_id, role_name],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role {} mcp server removed: {}", role_name, name);
            result.payload = json!({ "role": role_name, "mcpServers": servers });
        }
        ["/app_role", "copy", src_name, dst_name] => {
            let src = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, src_name],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, bool>(6)?)),
                ).map_err(|e| e.to_string())
            })?;
            let role = upsert_role(
                state.clone(),
                active_team_id.clone(),
                (*dst_name).to_string(),
                src.0,
                src.1,
                src.2,
                src.3,
                Some(src.4),
                Some(src.5),
                Some(src.6),
            )?;
            result.message = format!("role copied: {} -> {}", src_name, dst_name);
            result.payload = json!({ "role": role });
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
