use crate::acp::protocol as acp;
use std::collections::HashSet;

fn mcp_server_name(server: &acp::McpServer) -> Option<&str> {
    match server {
        acp::McpServer::Http(s) => Some(s.name.as_str()),
        acp::McpServer::Sse(s) => Some(s.name.as_str()),
        acp::McpServer::Stdio(s) => Some(s.name.as_str()),
        _ => None,
    }
}

pub(super) fn merge_mcp_servers(base: &mut Vec<acp::McpServer>, extras: Vec<acp::McpServer>) {
    let mut existing_names: HashSet<String> = base
        .iter()
        .filter_map(mcp_server_name)
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    for server in extras {
        let Some(name) = mcp_server_name(&server)
            .map(|raw| raw.trim().to_ascii_lowercase())
            .filter(|n| !n.is_empty())
        else {
            base.push(server);
            continue;
        };
        if existing_names.insert(name) {
            base.push(server);
        }
    }
}

pub(super) fn load_role_mcp_servers(
    state: &crate::types::AppState,
    role_name: &str,
    project_id: Option<&str>,
) -> Vec<acp::McpServer> {
    let role = crate::db::role::load_role_scoped(state, role_name, project_id)
        .ok()
        .flatten();
    let mut servers = role
        .as_ref()
        .map(|role| crate::db::global_mcp::get_enabled_mcp_for_role(state, &role.id))
        .unwrap_or_default();
    let role_servers = role
        .as_ref()
        .map(|r| crate::db::global_mcp::parse_mcp_server_list_json_compat(&r.mcp_servers_json))
        .unwrap_or_default();
    merge_mcp_servers(&mut servers, role_servers);
    servers
}
