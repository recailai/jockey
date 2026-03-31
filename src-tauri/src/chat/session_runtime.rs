use crate::db::app_session_role::load_app_session_role_state;
use crate::db::context::{list_shared_context_internal, sanitize_dynamic_item_name};
use crate::db::role::load_role;
use crate::db::session_context::app_session_role_scope;
use crate::types::AppState;

use super::RecentRoleChat;

pub(super) struct RoleRuntimeData {
    pub(super) runtime: String,
    pub(super) context_pairs: Vec<(String, String)>,
    pub(super) auto_approve: bool,
    pub(super) role_mode: Option<String>,
    pub(super) role_config: Vec<(String, String)>,
    pub(super) role_system_prompt: Option<String>,
    pub(super) context_log: Option<(usize, Option<String>)>,
    pub(super) mcp_servers: Vec<agent_client_protocol::McpServer>,
}

fn upsert_context_pair(context_pairs: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some(existing) = context_pairs.iter_mut().find(|(k, _)| k == key) {
        existing.1 = value;
    } else {
        context_pairs.push((key.to_string(), value));
    }
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

pub(super) fn load_role_runtime_data(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    assistant_runtime: &str,
    recent_chats_snapshot: Vec<RecentRoleChat>,
) -> Result<RoleRuntimeData, String> {
    let role_state = load_app_session_role_state(state, app_session_id, role_name)?;
    let role_data = load_role(state, role_name)?;

    let runtime = if role_name == "UnionAIAssistant" {
        assistant_runtime.to_string()
    } else {
        if role_state.is_none() && role_data.is_none() {
            return Err(format!("role not found: {role_name}"));
        }
        role_state
            .as_ref()
            .and_then(|row| row.runtime_kind.clone())
            .or_else(|| role_data.as_ref().map(|r| r.runtime_kind.clone()))
            .ok_or_else(|| format!("runtime not found for role: {role_name}"))?
    };

    let scope = app_session_role_scope(app_session_id, role_name);
    let entries = list_shared_context_internal(state, &scope).unwrap_or_default();
    let mut context_pairs: Vec<(String, String)> =
        entries.into_iter().map(|e| (e.key, e.value)).collect();

    let mut context_log = None;

    if role_name != "UnionAIAssistant" {
        let role_prompt = role_data
            .as_ref()
            .map(|r| r.system_prompt.clone())
            .unwrap_or_default();

        if !role_prompt.is_empty() {
            context_pairs.push(("role_prompt".to_string(), role_prompt));
        }
        // Only inject cross-role context on the first message to this role in the session.
        // If this role has already replied at least once, it already has its own history
        // in the ACP session — no need to keep prepending the handoff context every turn.
        let this_role_has_history = recent_chats_snapshot.iter().any(|c| c.role == role_name);

        let cross_role_chats: Vec<_> = recent_chats_snapshot
            .into_iter()
            .filter(|c| c.role != role_name)
            .collect();

        let inherited_cwd: Option<String> = cross_role_chats
            .iter()
            .rev()
            .find(|c| !c.cwd.is_empty())
            .map(|c| c.cwd.clone());

        if !cross_role_chats.is_empty() && !this_role_has_history {
            if let Ok(payload) = serde_json::to_string(&cross_role_chats) {
                upsert_context_pair(&mut context_pairs, "from_last_role_context", payload);
            }
            context_log = Some((cross_role_chats.len(), inherited_cwd.clone()));
        }
        if let Some(prev_cwd) = inherited_cwd {
            upsert_context_pair(&mut context_pairs, "cwd", prev_cwd);
        }
    }
    let auto_approve = role_data.as_ref().map(|r| r.auto_approve).unwrap_or(true);
    let role_mode = role_data.as_ref().and_then(|r| r.mode.clone());
    let mut role_config: Vec<(String, String)> = role_data
        .as_ref()
        .map(|r| parse_config_map(&r.config_options_json))
        .unwrap_or_default();

    if !role_config.iter().any(|(k, _)| k == "model") {
        let model = role_state
            .as_ref()
            .and_then(|r| r.model_override.clone())
            .or_else(|| role_data.as_ref().and_then(|r| r.model.clone()))
            .or_else(|| {
                context_pairs
                    .iter()
                    .find(|(k, _)| k == "model")
                    .map(|(_, v)| v.clone())
            });
        if let Some(model) = model {
            role_config.push(("model".to_string(), model));
        }
    }

    let role_system_prompt = role_data
        .as_ref()
        .map(|r| r.system_prompt.clone())
        .filter(|s| !s.is_empty());
    let mut mcp_servers = role_data
        .as_ref()
        .map(|r| &r.mcp_servers_json)
        .and_then(|raw| serde_json::from_str::<Vec<agent_client_protocol::McpServer>>(raw).ok())
        .unwrap_or_default();

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
                agent_client_protocol::McpServer::Http(s) => s.name.as_str(),
                agent_client_protocol::McpServer::Sse(s) => s.name.as_str(),
                agent_client_protocol::McpServer::Stdio(s) => s.name.as_str(),
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

    Ok(RoleRuntimeData {
        runtime,
        context_pairs,
        auto_approve,
        role_mode,
        role_config,
        role_system_prompt,
        context_log,
        mcp_servers,
    })
}
