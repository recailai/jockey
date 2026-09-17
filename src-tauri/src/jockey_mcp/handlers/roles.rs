use serde_json::{json, Value};

use crate::db::global_mcp::{
    delete_global_mcp_server, list_global_mcp_servers, upsert_global_mcp_server,
};
use crate::db::role::{delete_role_internal, list_all_roles, upsert_role};
use crate::types::AppState;

pub(crate) fn list_roles(state: &AppState) -> Result<Value, String> {
    let roles = list_all_roles(state)?;
    let out: Vec<Value> = roles
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "projectId": r.project_id,
                "roleName": r.role_name,
                "runtimeKind": r.runtime_kind,
                "runtimeProfileId": r.runtime_profile_id,
                "runtimeLaunchMethod": r.runtime_launch_method,
                "model": r.model,
                "mode": r.mode,
                "mcpServers": serde_json::from_str::<Value>(&r.mcp_servers_json).unwrap_or(json!([])),
                "configOptions": serde_json::from_str::<Value>(&r.config_options_json).unwrap_or(json!({})),
                "configOptionDefs": serde_json::from_str::<Value>(&r.config_option_defs_json).unwrap_or(json!([])),
                "autoApprove": r.auto_approve,
            })
        })
        .collect();
    Ok(json!(out))
}

pub(crate) fn get_role(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?;
    let project_id = params.get("projectId").and_then(|v| v.as_str());
    let role = crate::db::role::load_role_scoped(state, role_name, project_id)?
        .ok_or_else(|| format!("role not found: {role_name}"))?;
    Ok(json!({
        "id": role.id,
        "projectId": role.project_id,
        "roleName": role.role_name,
        "runtimeKind": role.runtime_kind,
        "runtimeProfileId": role.runtime_profile_id,
        "runtimeLaunchMethod": role.runtime_launch_method,
        "systemPrompt": role.system_prompt,
        "model": role.model,
        "mode": role.mode,
        "mcpServers": serde_json::from_str::<Value>(&role.mcp_servers_json).unwrap_or(json!([])),
        "configOptions": serde_json::from_str::<Value>(&role.config_options_json).unwrap_or(json!({})),
        "configOptionDefs": serde_json::from_str::<Value>(&role.config_option_defs_json).unwrap_or(json!([])),
        "autoApprove": role.auto_approve,
    }))
}

pub(crate) fn upsert_role_handler(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?
        .to_string();
    let runtime_kind = params
        .get("runtimeKind")
        .and_then(|v| v.as_str())
        .unwrap_or("claude-native")
        .to_string();
    let runtime_profile_id = params
        .get("runtimeProfileId")
        .and_then(|v| v.as_str())
        .unwrap_or(&runtime_kind)
        .to_string();
    let system_prompt = params
        .get("systemPrompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = params
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let auto_approve = params.get("autoApprove").and_then(|v| v.as_bool());
    let mcp_servers_json = params
        .get("mcpServersJson")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let config_options_json = params
        .get("configOptionsJson")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let config_option_defs_json = params
        .get("configOptionDefsJson")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let project_id = params
        .get("projectId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    upsert_role(
        state,
        role_name.clone(),
        runtime_profile_id,
        system_prompt,
        model,
        mode,
        mcp_servers_json,
        config_options_json,
        config_option_defs_json,
        auto_approve,
        project_id,
    )?;
    Ok(json!(format!(
        "Role '{role_name}' saved (runtime: {runtime_kind})"
    )))
}

pub(crate) fn delete_role_handler(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?;
    delete_role_internal(state, role_name, None, None)?;
    Ok(json!(format!("Role '{role_name}' deleted")))
}

pub(crate) fn list_mcp_servers(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params.get("roleName").and_then(|v| v.as_str());
    let global = list_global_mcp_servers(state)?;
    let global_out: Vec<Value> = global
        .iter()
        .map(|g| {
            json!({
                "name": g.name,
                "config": serde_json::from_str::<Value>(&g.config_json).unwrap_or(json!({})),
                "isBuiltin": g.is_builtin,
            })
        })
        .collect();

    let roles = list_all_roles(state)?;
    if let Some(rn) = role_name {
        let project_id = params.get("projectId").and_then(|v| v.as_str());
        let role = crate::db::role::load_role_scoped(state, rn, project_id)?
            .ok_or_else(|| format!("role not found: {rn}"))?;
        let role_entries = crate::db::global_mcp::list_role_mcp_servers(state, &role.id)?;
        let enabled_mcp: Vec<Value> = role_entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| {
                json!({
                    "name": e.mcp_server_name,
                    "config": serde_json::from_str::<Value>(&e.config_json).unwrap_or(json!({})),
                    "isBuiltin": e.is_builtin,
                })
            })
            .collect();
        let role_servers: Value = serde_json::from_str(&role.mcp_servers_json).unwrap_or(json!([]));
        return Ok(json!({
            "globalRegistry": global_out,
            "roleServers": role_servers,
            "enabledGlobalMcp": enabled_mcp,
        }));
    }

    let roles_out: Vec<Value> = roles
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "projectId": r.project_id,
                "roleName": r.role_name,
                "mcpServers": serde_json::from_str::<Value>(&r.mcp_servers_json).unwrap_or(json!([]))
            })
        })
        .collect();
    Ok(json!({
        "globalRegistry": global_out,
        "roles": roles_out,
    }))
}

pub(crate) fn add_mcp_to_role(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?;
    let project_id = params.get("projectId").and_then(|v| v.as_str());
    let server = params.get("server").ok_or("server object is required")?;
    let new_name = server.get("name").and_then(|v| v.as_str()).unwrap_or("");

    let role = crate::db::role::load_role_scoped(state, role_name, project_id)?
        .ok_or_else(|| format!("role not found: {role_name}"))?;
    let mut servers: Vec<Value> =
        serde_json::from_str(&role.mcp_servers_json).map_err(|e| e.to_string())?;

    if servers
        .iter()
        .any(|s| s.get("name").and_then(|v| v.as_str()) == Some(new_name))
    {
        return Err(format!(
            "MCP server '{new_name}' already exists on role '{role_name}'"
        ));
    }

    servers.push(server.clone());
    let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;

    upsert_role(
        state,
        role.role_name,
        role.runtime_kind,
        role.system_prompt,
        role.model,
        role.mode,
        Some(updated),
        Some(role.config_options_json),
        Some(role.config_option_defs_json),
        Some(role.auto_approve),
        role.project_id,
    )?;
    let _ = crate::db::global_mcp::set_role_mcp_enabled(state, &role.id, new_name, true);

    Ok(json!(format!(
        "Added MCP server '{new_name}' to role '{role_name}'. Total servers: {}",
        servers.len()
    )))
}

pub(crate) fn remove_mcp_from_role(state: &AppState, params: Value) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?;
    let project_id = params.get("projectId").and_then(|v| v.as_str());
    let server_name = params
        .get("serverName")
        .and_then(|v| v.as_str())
        .ok_or("serverName is required")?;

    let role = crate::db::role::load_role_scoped(state, role_name, project_id)?
        .ok_or_else(|| format!("role not found: {role_name}"))?;
    let mut servers: Vec<Value> =
        serde_json::from_str(&role.mcp_servers_json).map_err(|e| e.to_string())?;
    let before = servers.len();
    servers.retain(|s| s.get("name").and_then(|v| v.as_str()) != Some(server_name));
    if servers.len() == before {
        return Err(format!(
            "MCP server '{server_name}' not found on role '{role_name}'"
        ));
    }

    let updated = serde_json::to_string(&servers).map_err(|e| e.to_string())?;
    upsert_role(
        state,
        role.role_name,
        role.runtime_kind,
        role.system_prompt,
        role.model,
        role.mode,
        Some(updated),
        Some(role.config_options_json),
        Some(role.config_option_defs_json),
        Some(role.auto_approve),
        role.project_id,
    )?;
    let _ = crate::db::global_mcp::set_role_mcp_enabled(state, &role.id, server_name, false);

    Ok(json!(format!(
        "Removed MCP server '{server_name}' from role '{role_name}'. Remaining: {}",
        servers.len()
    )))
}

pub(crate) fn upsert_global_mcp(state: &AppState, params: Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("name is required")?;
    let config = params.get("config").ok_or("config object is required")?;
    let config_json = serde_json::to_string(config).map_err(|e| e.to_string())?;
    upsert_global_mcp_server(state, name, &config_json, false)?;
    Ok(json!(format!("Global MCP server '{name}' saved")))
}

pub(crate) fn delete_global_mcp(state: &AppState, params: Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("name is required")?;
    delete_global_mcp_server(state, name)?;
    Ok(json!(format!("Global MCP server '{name}' deleted")))
}

pub(crate) async fn invoke_role(
    state: &AppState,
    app: &tauri::AppHandle,
    params: Value,
) -> Result<Value, String> {
    let role_name = params
        .get("roleName")
        .and_then(|v| v.as_str())
        .ok_or("roleName is required")?;
    let prompt = params
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("prompt is required")?;
    let app_session_id = params
        .get("appSessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "mcp_subagent_session".to_string());

    let project_id = crate::db::app_session::get_app_session_project_id(state, &app_session_id);
    let role = crate::db::role::load_role_scoped(state, role_name, project_id.as_deref())?
        .ok_or_else(|| format!("target role not found: {role_name}"))?;

    let cwd = crate::db::app_session::get_app_session_cwd(state, &app_session_id)
        .unwrap_or_else(crate::resolve_chat_cwd);

    let role_system_prompt = if !role.system_prompt.trim().is_empty() {
        Some(role.system_prompt.as_str())
    } else {
        None
    };

    let enabled_rules =
        crate::db::rule::get_enabled_rules_for_role(state, &role.id).unwrap_or_default();
    let prepared = crate::chat::prompt_builder::build_prepared_prompt(
        role_system_prompt,
        &enabled_rules,
        &[],
        prompt,
    );

    let mcp_servers: Vec<crate::acp::protocol::McpServer> = {
        let mut servers = crate::db::global_mcp::get_enabled_mcp_for_role(state, &role.id);
        let role_servers =
            crate::db::global_mcp::parse_mcp_server_list_json_compat(&role.mcp_servers_json);
        let existing_names: std::collections::HashSet<String> = servers
            .iter()
            .map(|s| match s {
                crate::acp::protocol::McpServer::Http(h) => h.name.clone(),
                crate::acp::protocol::McpServer::Sse(e) => e.name.clone(),
                crate::acp::protocol::McpServer::Stdio(s) => s.name.clone(),
                _ => String::new(),
            })
            .collect();
        for rs in role_servers {
            let name = match &rs {
                crate::acp::protocol::McpServer::Http(h) => &h.name,
                crate::acp::protocol::McpServer::Sse(e) => &e.name,
                crate::acp::protocol::McpServer::Stdio(s) => &s.name,
                _ => continue,
            };
            if !existing_names.contains(name) {
                servers.push(rs);
            }
        }
        servers
    };

    let role_config: Vec<(String, String)> =
        serde_json::from_str::<serde_json::Value>(&role.config_options_json)
            .ok()
            .and_then(|v| {
                v.as_object().map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
            })
            .unwrap_or_default();

    let auto_approve = role.auto_approve;
    let role_mode = role.mode;

    let res = crate::acp::execute_runtime(
        &role.runtime_kind,
        role_name,
        &prepared,
        &[],
        &[],
        &cwd,
        app,
        auto_approve,
        mcp_servers,
        role_mode,
        role_config,
        Some((state, &app_session_id)),
        &app_session_id,
    )
    .await;

    if !res.ok && res.output.trim().is_empty() {
        return Err(format!(
            "role execution failed: {}",
            res.meta
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        ));
    }

    let output = if res.output.trim().is_empty() && !res.deltas.is_empty() {
        res.deltas.join("")
    } else {
        res.output
    };

    Ok(json!({
        "role": role_name,
        "status": if res.ok { "success" } else { "partial_or_error" },
        "output": output
    }))
}
