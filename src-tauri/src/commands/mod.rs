pub(crate) mod completion;

use crate::types::*;
use crate::db::{with_db, get_state, team_exists, ensure_default_team_id};
use crate::db::context::*;
use crate::db::role::*;
use crate::db::session::*;
use crate::db::workflow::*;
use crate::assistant::normalize_runtime_key;
use crate::{now_ms, resolve_chat_cwd, acp, build_unionai_tool_prompt};
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
    if !trimmed.starts_with('/') {
        return Err("chat set command must start with /".to_string());
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
        ["/help"] => {
            result.message = "command list".to_string();
            result.payload = json!({ "help": build_unionai_tool_prompt() });
        }
        ["/assistant", "list"] => {
            let assistants = crate::assistant::assistant_catalog();
            result.message = "assistant list".to_string();
            result.payload = json!({ "assistants": assistants });
        }
        ["/assistant", "select", runtime] => match normalize_runtime_key(runtime) {
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
        ["/model", "list"] => {
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
        ["/model", "add", model] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "model", model)?;
            result.message = format!("model added: {}", name);
            result.payload = json!({ "model": name });
        }
        ["/model", "remove", model] => {
            let removed = remove_dynamic_catalog_item(get_state(&state), "model", model)?;
            result.ok = removed;
            result.message = if removed {
                format!("model removed: {}", model)
            } else {
                format!("model not found: {}", model)
            };
            result.payload = json!({ "model": model, "removed": removed });
        }
        ["/model", "select", "role", role_name, model] => {
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
        ["/model", "select", model] => {
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
        ["/model", "get", "role", role_name] => {
            let entry = get_shared_context(
                state.clone(),
                context_scope_for_role(role_name),
                "model".to_string(),
            )?;
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            result.message = format!("model fetched for role {}", role_name);
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/model", "get"] => {
            let entry = get_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                "model".to_string(),
            )?;
            let runtime = resolve_model_runtime(result.selected_assistant.as_deref());
            result.message = "model fetched".to_string();
            result.payload = json!({ "entry": entry, "runtime": runtime });
        }
        ["/model", "clear", "role", role_name] => {
            let scope = context_scope_for_role(role_name);
            clear_shared_context_internal(get_state(&state), &scope, "model")?;
            result.message = format!("model cleared for role {}", role_name);
            result.payload = json!({ "role": role_name });
        }
        ["/model", "clear"] => {
            clear_shared_context_internal(get_state(&state), "assistant:main", "model")?;
            result.message = "model cleared".to_string();
            result.payload = json!({});
        }
        ["/mcp", "list"] => {
            let catalog = list_dynamic_catalog(get_state(&state), "mcp")?;
            let enabled = list_enabled_feature_flags(get_state(&state), "mcp:");
            result.message = format!("{} MCP servers enabled", enabled.len());
            result.payload = json!({ "catalog": catalog, "enabled": enabled });
        }
        ["/mcp", "add", server] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "mcp", server)?;
            result.message = format!("mcp server added: {}", name);
            result.payload = json!({ "server": name });
        }
        ["/mcp", "remove", server] => {
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
        ["/mcp", mode, server] if *mode == "enable" || *mode == "disable" => {
            let name = sanitize_dynamic_item_name(server)
                .ok_or_else(|| format!("invalid mcp server name: {}", server))?;
            if !dynamic_catalog_contains(get_state(&state), "mcp", &name)? {
                let supported = list_dynamic_catalog(get_state(&state), "mcp")?;
                result.ok = false;
                result.message = format!("unsupported mcp server: {}", server);
                result.payload = json!({ "supported": supported });
            } else {
                let value = if *mode == "enable" { "enabled" } else { "disabled" };
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
        ["/skill", "list"] => {
            let catalog = list_dynamic_catalog(get_state(&state), "skill")?;
            let enabled = list_enabled_feature_flags(get_state(&state), "skill:");
            result.message = format!("{} skills enabled", enabled.len());
            result.payload = json!({ "catalog": catalog, "enabled": enabled });
        }
        ["/skill", "add", skill] => {
            let name = upsert_dynamic_catalog_item(get_state(&state), "skill", skill)?;
            result.message = format!("skill added: {}", name);
            result.payload = json!({ "skill": name });
        }
        ["/skill", "remove", skill] => {
            let name = sanitize_dynamic_item_name(skill)
                .ok_or_else(|| format!("invalid skill name: {}", skill))?;
            let removed = remove_dynamic_catalog_item(get_state(&state), "skill", &name)?;
            if removed {
                clear_shared_context_internal(
                    get_state(&state),
                    "assistant:main",
                    &format!("skill:{name}"),
                )?;
            }
            result.ok = removed;
            result.message = if removed {
                format!("skill removed: {}", name)
            } else {
                format!("skill not found: {}", name)
            };
            result.payload = json!({ "skill": name, "removed": removed });
        }
        ["/skill", mode, skill] if *mode == "enable" || *mode == "disable" => {
            let name = sanitize_dynamic_item_name(skill)
                .ok_or_else(|| format!("invalid skill name: {}", skill))?;
            if !dynamic_catalog_contains(get_state(&state), "skill", &name)? {
                let supported = list_dynamic_catalog(get_state(&state), "skill")?;
                result.ok = false;
                result.message = format!("unsupported skill: {}", skill);
                result.payload = json!({ "supported": supported });
            } else {
                let value = if *mode == "enable" { "enabled" } else { "disabled" };
                let entry = set_shared_context(
                    state.clone(),
                    "assistant:main".to_string(),
                    format!("skill:{name}"),
                    value.to_string(),
                )?;
                result.message = format!("skill {}: {}", mode, name);
                result.payload = json!({ "entry": entry });
            }
        }
        ["/init", runtime_kind] | ["/init", _, runtime_kind] => {
            let normalized_runtime = normalize_runtime_key(runtime_kind)
                .map(|v| v.to_string())
                .unwrap_or_else(|| (*runtime_kind).to_string());
            let workflow = ensure_quick_workflow(
                state.clone(),
                &active_team_id,
                &normalized_runtime,
            )?;
            result.message = format!("initialized quick workflow: {}", workflow.name);
            result.selected_assistant = Some(normalized_runtime.clone());
            result.payload = json!({ "workspaceId": active_team_id.clone(), "workflow": workflow });
        }
        ["/run", prompt @ ..] => {
            let mut auto_created: Option<Workflow> = None;
            let workflow_id = if let Some(id) = latest_workflow_id(state.clone(), &active_team_id)?
            {
                id
            } else {
                let runtime_hint = result
                    .selected_assistant
                    .clone()
                    .unwrap_or_else(|| "mock".to_string());
                let workflow =
                    ensure_quick_workflow(state.clone(), &active_team_id, &runtime_hint)?;
                let id = workflow.id.clone();
                auto_created = Some(workflow);
                id
            };
            let prompt_text = if prompt.is_empty() {
                "run".to_string()
            } else {
                prompt.join(" ")
            };
            let session = start_workflow(
                app.clone(),
                state.clone(),
                StartWorkflowInput {
                    team_id: active_team_id.clone(),
                    workflow_id,
                    initial_prompt: prompt_text,
                },
            )
            .await?;
            result.message = if auto_created.is_some() {
                format!("run started with auto-created quick workflow: {}", session.id)
            } else {
                format!("run started: {}", session.id)
            };
            result.session_id = Some(session.id.clone());
            result.payload = json!({
                "session": session,
                "autoCreatedWorkflow": auto_created
            });
        }
        ["/team", ..] => {
            result.ok = false;
            result.message = "workspace commands are managed automatically.".to_string();
            result.payload = json!({});
        }
        ["/role", "list"] => {
            let roles = list_roles_for_team(get_state(&state), &active_team_id)?;
            result.message = format!("{} roles", roles.len());
            result.payload = json!({ "roles": roles });
        }
        ["/role", "bind", role_name, runtime_kind, prompt @ ..] => {
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
                acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &prewarm_cwd).await;
            });
        }
        ["/role", "prompt", role_name, prompt @ ..] => {
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
        ["/role", "delete", role_name] => {
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
        ["/role", "edit", role_name, "model", value @ ..] => {
            let model_value = if value.is_empty() { None } else { Some(value.join(" ")) };
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
        ["/role", "edit", role_name, "mode", value] => {
            let mode_value = if *value == "none" || *value == "clear" { None } else { Some((*value).to_string()) };
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
        ["/role", "edit", role_name, "auto-approve", value] => {
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
        ["/role", "edit", role_name, "mcp-add", server_json @ ..] => {
            let json_str = server_json.join(" ");
            let current = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT mcp_servers_json FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, role_name],
                    |row| row.get::<_, String>(0),
                ).map_err(|e| e.to_string())
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
        ["/role", "edit", role_name, "mcp-remove", name] => {
            let current = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT mcp_servers_json FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, role_name],
                    |row| row.get::<_, String>(0),
                ).map_err(|e| e.to_string())
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
        ["/role", "copy", src_name, dst_name] => {
            let src = with_db(get_state(&state), |conn| {
                conn.query_row(
                    "SELECT runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&active_team_id, src_name],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, bool>(6)?)),
                ).map_err(|e| e.to_string())
            })?;
            let role = upsert_role(state.clone(), active_team_id.clone(), (*dst_name).to_string(), src.0, src.1, src.2, src.3, Some(src.4), Some(src.5), Some(src.6))?;
            result.message = format!("role copied: {} -> {}", src_name, dst_name);
            result.payload = json!({ "role": role });
        }
        ["/workflow", "list"] => {
            let workflows = list_workflows(state.clone(), active_team_id.clone())?;
            result.message = format!("{} workflows", workflows.len());
            result.payload = json!({ "workflows": workflows });
        }
        ["/workflow", "create", name, steps_csv] => {
            let steps = steps_csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let wf = create_workflow(
                state.clone(),
                active_team_id.clone(),
                (*name).to_string(),
                steps,
            )?;
            result.message = format!("workflow created: {}", wf.name);
            result.payload = json!({ "workflow": wf });
        }
        ["/workflow", "update", workflow_ref, steps_csv] => {
            let workflow_id = resolve_workflow_id(state.clone(), &active_team_id, workflow_ref)?;
            let steps = steps_csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if steps.is_empty() {
                return Err("workflow steps cannot be empty".to_string());
            }
            let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE workflows SET steps_json = ?1, updated_at = ?2 WHERE id = ?3",
                    params![steps_json, now_ms(), &workflow_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("workflow updated: {}", workflow_id);
            result.payload = json!({ "workflowId": workflow_id, "steps": steps });
        }
        ["/workflow", "delete", workflow_ref] => {
            let workflow_id = resolve_workflow_id(state.clone(), &active_team_id, workflow_ref)?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM session_events WHERE session_id IN (SELECT id FROM sessions WHERE workflow_id = ?1)",
                    params![&workflow_id],
                )
                .map_err(|e| e.to_string())?;
                conn.execute(
                    "DELETE FROM sessions WHERE workflow_id = ?1",
                    params![&workflow_id],
                )
                .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM workflows WHERE id = ?1", params![&workflow_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("workflow deleted: {}", workflow_id);
            result.payload = json!({ "workflowId": workflow_id });
        }
        ["/workflow", "start", workflow_ref, prompt @ ..] => {
            let workflow_id = resolve_workflow_id(state.clone(), &active_team_id, workflow_ref)?;
            let prompt_text = if prompt.is_empty() {
                "start".to_string()
            } else {
                prompt.join(" ")
            };
            let session = start_workflow(
                app.clone(),
                state.clone(),
                StartWorkflowInput {
                    team_id: active_team_id.clone(),
                    workflow_id: workflow_id.clone(),
                    initial_prompt: prompt_text,
                },
            )
            .await?;
            result.message = format!("workflow started: {}", session.id);
            result.session_id = Some(session.id.clone());
            result.payload = json!({ "session": session });
        }
        ["/session", "list"] => {
            let sessions = list_sessions(state.clone(), active_team_id.clone())?;
            result.message = format!("{} sessions", sessions.len());
            result.payload = json!({ "sessions": sessions });
        }
        ["/session", "events", session_id] => {
            let events =
                list_session_events(state.clone(), (*session_id).to_string(), None, Some(300))?;
            result.message = format!("{} session events", events.len());
            result.payload = json!({ "events": events, "sessionId": session_id });
        }
        ["/session", "stop", session_id] => {
            update_session_status(get_state(&state), session_id, "stopped")?;
            result.message = format!("session stopped: {}", session_id);
            result.payload = json!({ "sessionId": session_id });
        }
        ["/session", "reset", "assistant"] => {
            let runtime = result
                .selected_assistant
                .clone()
                .unwrap_or_else(|| "mock".to_string());
            let normalized = normalize_runtime_key(&runtime).unwrap_or("mock");
            acp::reset_slot(normalized, "UnionAIAssistant");
            result.message = format!("assistant session reset: {}", normalized);
            result.payload = json!({ "assistant": normalized });
        }
        ["/session", "reset", "role", role_name] => {
            let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
            acp::reset_slot(&runtime, role_name);
            result.message = format!("role session reset: {}", role_name);
            result.payload = json!({ "role": role_name, "runtime": runtime });
        }
        ["/session", "reset", target] if target.starts_with("role:") => {
            let role_name = target.trim_start_matches("role:");
            if role_name.is_empty() {
                result.ok = false;
                result.message = "missing role name".to_string();
            } else {
                let runtime = resolve_role_runtime(state.clone(), &active_team_id, role_name)?;
                acp::reset_slot(&runtime, role_name);
                result.message = format!("role session reset: {}", role_name);
                result.payload = json!({ "role": role_name, "runtime": runtime });
            }
        }
        ["/session", "delete", session_id] => {
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM session_events WHERE session_id = ?1",
                    params![session_id],
                )
                .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("session deleted: {}", session_id);
            result.payload = json!({ "sessionId": session_id });
        }
        ["/context", "list"] => {
            let entries =
                list_shared_context(state.clone(), "assistant:main".to_string())?;
            result.message = format!("{} context entries", entries.len());
            result.payload = json!({ "entries": entries });
        }
        ["/context", "list", "role", role_name] => {
            let entries =
                list_shared_context(state.clone(), context_scope_for_role(role_name))?;
            result.message = format!("{} context entries", entries.len());
            result.payload = json!({ "entries": entries });
        }
        ["/context", "set", "role", role_name, key, value @ ..] => {
            let text = value.join(" ");
            let entry = set_shared_context(
                state.clone(),
                context_scope_for_role(role_name),
                (*key).to_string(),
                text,
            )?;
            result.message = format!("context set for role {}: {}", role_name, entry.key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "set", key, value @ ..] => {
            let text = value.join(" ");
            let entry = set_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                (*key).to_string(),
                text,
            )?;
            result.message = format!("context set: {}", entry.key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "get", "role", role_name, key] => {
            let entry = get_shared_context(
                state.clone(),
                context_scope_for_role(role_name),
                (*key).to_string(),
            )?;
            result.message = format!("context fetched for role {}: {}", role_name, key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "get", key] => {
            let entry = get_shared_context(
                state.clone(),
                "assistant:main".to_string(),
                (*key).to_string(),
            )?;
            result.message = format!("context fetched: {}", key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "delete", "role", role_name, key] => {
            let scope = context_scope_for_role(role_name);
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM shared_context_snapshots WHERE team_id = ?1 AND key = ?2",
                    params![&scope, key],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            get_state(&state)
                .shared_context
                .remove(&shared_key(&scope, key));
            result.message = format!("context deleted for role {}: {}", role_name, key);
            result.payload = json!({ "key": key, "role": role_name });
        }
        ["/context", "delete", key] => {
            let scope = "assistant:main".to_string();
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM shared_context_snapshots WHERE team_id = ?1 AND key = ?2",
                    params![&scope, key],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            get_state(&state)
                .shared_context
                .remove(&shared_key(&scope, key));
            result.message = format!("context deleted: {}", key);
            result.payload = json!({ "key": key });
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
