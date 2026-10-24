use crate::acp::protocol as acp;
use crate::db::context::{list_shared_context_internal, sanitize_dynamic_item_name};
use crate::db::role::load_role_scoped;
use crate::db::session_context::app_session_role_scope;
use crate::runtime_kind::RuntimeKind;
use crate::types::AppState;

pub(super) struct RoleRuntimeData {
    pub(super) runtime: String,
    pub(super) auto_approve: bool,
    pub(super) role_mode: Option<String>,
    pub(super) role_config: Vec<(String, String)>,
    pub(super) mcp_servers: Vec<acp::McpServer>,
}

fn parse_config_map(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.as_object().map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn normalize_runtime_key(runtime: &str) -> String {
    RuntimeKind::from_str(runtime)
        .map(|k| k.runtime_key().to_string())
        .unwrap_or_else(|| runtime.trim().to_ascii_lowercase())
}

fn sanitize_model_for_runtime(runtime: &str, selected_model: &str) -> Option<String> {
    let trimmed = selected_model.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized_runtime = RuntimeKind::from_str(runtime)
        .map(|k| k.runtime_key())
        .unwrap_or(runtime);
    let discovered = crate::acp::list_discovered_models(normalized_runtime);
    if discovered.is_empty() {
        return Some(trimmed.to_string());
    }
    if let Some(canonical) = discovered
        .iter()
        .find(|m| m.trim().eq_ignore_ascii_case(trimmed))
    {
        return Some(canonical.clone());
    }
    Some(trimmed.to_string())
}

fn resolve_model(
    role_data: Option<&crate::types::Role>,
    role_state: Option<&crate::db::app_session_role::AppSessionRoleState>,
    project_agent_config: &[(String, String)],
    _runtime_key: &str,
    runtime: &str,
    context_pairs: &[(String, String)],
) -> Option<String> {
    let model_override = role_state.as_ref().and_then(|r| r.model_override.clone());

    if let Some(model) = model_override.and_then(|m| sanitize_model_for_runtime(runtime, &m)) {
        return Some(model);
    }

    let session_cfg_model = role_state
        .as_ref()
        .and_then(|r| r.config_options_json.as_deref())
        .and_then(|raw| {
            parse_config_map(raw)
                .into_iter()
                .find(|(k, _)| k == "model")
                .map(|(_, v)| v)
        });
    if let Some(model) = session_cfg_model.and_then(|m| sanitize_model_for_runtime(runtime, &m)) {
        return Some(model);
    }

    let project_model = project_agent_config
        .iter()
        .find(|(k, _)| k == "model")
        .map(|(_, v)| v.clone());
    if let Some(model) = project_model.and_then(|m| sanitize_model_for_runtime(runtime, &m)) {
        return Some(model);
    }

    let role_cfg_model = role_data
        .map(|r| parse_config_map(&r.config_options_json))
        .and_then(|cfg| cfg.into_iter().find(|(k, _)| k == "model").map(|(_, v)| v));
    if let Some(model) = role_cfg_model.and_then(|m| sanitize_model_for_runtime(runtime, &m)) {
        return Some(model);
    }

    let role_model = role_data.and_then(|r| r.model.clone());
    if let Some(model) = role_model.and_then(|m| sanitize_model_for_runtime(runtime, &m)) {
        return Some(model);
    }

    context_pairs
        .iter()
        .find(|(k, _)| k == "model")
        .and_then(|(_, v)| sanitize_model_for_runtime(runtime, v))
}

pub(super) fn load_role_runtime_data(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    assistant_runtime: &str,
) -> Result<RoleRuntimeData, String> {
    let project_id = crate::db::app_session::get_app_session_project_id(state, app_session_id);
    let role_data = load_role_scoped(state, role_name, project_id.as_deref())?;

    // One rule for every caller (see `engine::sessions::resolve_role_runtime`): the request's
    // selection applies to the active persona; any other persona resolves its own engine.
    let runtime = crate::engine::sessions::resolve_role_runtime(
        state,
        app_session_id,
        role_name,
        Some(assistant_runtime),
        None,
    )
    .unwrap_or_default();
    let runtime = normalize_runtime_key(&runtime);

    // Keep the authoritative runtime and its provider-session slot together. This makes a
    // switch durable even when the frontend has not finished its fire-and-forget binding call.
    let _ = crate::db::app_session_role::bind_app_session_role_runtime(
        state,
        app_session_id,
        role_name,
        &runtime,
    );

    // This is the real send path (the only caller of `load_role_runtime_data`), so this is
    // the right moment — and the only moment — to register the persona under this project.
    // Best-effort: a registration failure must never block the turn from proceeding, and it
    // must never overwrite a runtime the project already pinned (see `ensure_..._registered`).
    let _ = crate::db::project_agent::ensure_project_agent_registered(
        state,
        project_id.as_deref(),
        role_name,
        &runtime,
    );

    let scope = app_session_role_scope(app_session_id, role_name);
    let entries = list_shared_context_internal(state, &scope).unwrap_or_default();
    // Shared context entries stay configuration only (model hint, `mcp:<name>` flags);
    // they are not sent to the agent. History is available on demand via the MCP tools.
    let context_pairs: Vec<(String, String)> =
        entries.into_iter().map(|e| (e.key, e.value)).collect();
    let auto_approve = role_data.as_ref().map(|r| r.auto_approve).unwrap_or(false);
    let runtime_key = normalize_runtime_key(&runtime);
    let role_state = crate::db::app_session_role::load_app_session_role_runtime_state(
        state,
        app_session_id,
        role_name,
        &runtime_key,
    )?;
    // Layering, lowest to highest: persona default -> project agent config -> session override.
    // The project layer is what makes "this repo runs on opus" survive opening a new session.
    // Knobs belong to the engine that will run this turn, not to whichever one is pinned —
    // those differ while a session temporarily overrides the pin.
    let project_agent_config = crate::db::project_agent::load_project_agent_options(
        state,
        project_id.as_deref(),
        role_name,
        &runtime,
    )
    .unwrap_or_default();
    let role_mode = role_state
        .as_ref()
        .and_then(|r| r.mode_override.clone())
        .or_else(|| {
            project_agent_config
                .iter()
                .find(|(k, _)| k == "mode")
                .map(|(_, v)| v.clone())
        })
        .or_else(|| role_data.as_ref().and_then(|r| r.mode.clone()));
    let mut role_config: Vec<(String, String)> = role_data
        .as_ref()
        .map(|r| parse_config_map(&r.config_options_json))
        .unwrap_or_default();
    for (key, value) in &project_agent_config {
        if let Some(existing) = role_config.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value.clone();
        } else {
            role_config.push((key.clone(), value.clone()));
        }
    }
    if let Some(session_cfg) = role_state
        .as_ref()
        .and_then(|r| r.config_options_json.as_deref())
    {
        for (key, value) in parse_config_map(session_cfg) {
            if value.trim().is_empty() {
                role_config.retain(|(k, _)| k != &key);
                continue;
            }
            if let Some(existing) = role_config.iter_mut().find(|(k, _)| k == &key) {
                existing.1 = value;
            } else {
                role_config.push((key, value));
            }
        }
    }

    role_config.retain(|(k, _)| k != "model");
    if let Some(model) = resolve_model(
        role_data.as_ref(),
        role_state.as_ref(),
        &project_agent_config,
        &runtime_key,
        &runtime,
        &context_pairs,
    ) {
        role_config.push(("model".to_string(), model));
    }

    let mut mcp_servers: Vec<acp::McpServer> = {
        let mut servers = role_data
            .as_ref()
            .map(|role| crate::db::global_mcp::get_enabled_mcp_for_role(state, &role.id))
            .unwrap_or_default();
        let role_servers = role_data
            .as_ref()
            .map(|r| &r.mcp_servers_json)
            .map(|raw| crate::db::global_mcp::parse_mcp_server_list_json_compat(raw))
            .unwrap_or_default();
        let existing_names: std::collections::HashSet<String> = servers
            .iter()
            .map(|s| match s {
                acp::McpServer::Http(h) => h.name.clone(),
                acp::McpServer::Sse(e) => e.name.clone(),
                acp::McpServer::Stdio(d) => d.name.clone(),
                _ => String::new(),
            })
            .collect();
        for s in role_servers {
            let name = match &s {
                acp::McpServer::Http(h) => h.name.as_str(),
                acp::McpServer::Sse(e) => e.name.as_str(),
                acp::McpServer::Stdio(d) => d.name.as_str(),
                _ => "",
            };
            if !name.is_empty() && !existing_names.contains(name) {
                servers.push(s);
            }
        }
        servers
    };

    if matches!(
        runtime_key.as_str(),
        "codex-cli" | "pi-cli" | "antigravity-cli"
    ) {
        mcp_servers.retain(|server| {
            let name = match server {
                acp::McpServer::Http(h) => h.name.as_str(),
                acp::McpServer::Sse(e) => e.name.as_str(),
                acp::McpServer::Stdio(d) => d.name.as_str(),
                _ => "",
            };
            name != "jockey"
        });
    }

    // Respect per-session MCP feature flags when present (mcp:<name>=enabled/disabled).
    // If no flag is set for this scope, keep default role/session MCP server list.
    let mcp_flags: std::collections::HashMap<String, String> = context_pairs
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("mcp:")
                .map(|name| (name.to_ascii_lowercase(), v.to_ascii_lowercase()))
        })
        .collect();
    if !mcp_flags.is_empty() {
        mcp_servers.retain(|server| {
            let server_name = match server {
                acp::McpServer::Http(s) => s.name.as_str(),
                acp::McpServer::Sse(s) => s.name.as_str(),
                acp::McpServer::Stdio(s) => s.name.as_str(),
                _ => "",
            };
            let normalized_name = sanitize_dynamic_item_name(server_name)
                .unwrap_or_else(|| server_name.trim().to_ascii_lowercase());
            match mcp_flags.get(&normalized_name) {
                Some(flag) => flag == "enabled",
                None => true,
            }
        });
    }

    crate::jockey_mcp::binding::bind_jockey_servers(&mut mcp_servers, app_session_id, role_name);

    Ok(RoleRuntimeData {
        runtime,
        auto_approve,
        role_mode,
        role_config,
        mcp_servers,
    })
}
