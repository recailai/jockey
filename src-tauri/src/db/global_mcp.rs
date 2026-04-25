use agent_client_protocol as acp;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use tauri::State;

use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::AppState;

static GLOBAL_MCP_CACHE: OnceLock<RwLock<HashMap<usize, Vec<GlobalMcpEntry>>>> = OnceLock::new();
static GLOBAL_MCP_ACP_CACHE: OnceLock<RwLock<HashMap<usize, Vec<acp::McpServer>>>> =
    OnceLock::new();

fn global_mcp_cache() -> &'static RwLock<HashMap<usize, Vec<GlobalMcpEntry>>> {
    GLOBAL_MCP_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn global_mcp_acp_cache() -> &'static RwLock<HashMap<usize, Vec<acp::McpServer>>> {
    GLOBAL_MCP_ACP_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn cache_key(state: &AppState) -> usize {
    state.db.cache_key()
}

fn invalidate_global_mcp_cache(state: &AppState) {
    let key = cache_key(state);
    if let Ok(mut w) = global_mcp_cache().write() {
        w.remove(&key);
    }
    if let Ok(mut w) = global_mcp_acp_cache().write() {
        w.remove(&key);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalMcpEntry {
    pub name: String,
    pub config_json: String,
    pub is_builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleMcpEntry {
    pub mcp_server_name: String,
    pub config_json: String,
    pub is_builtin: bool,
    pub enabled: bool,
}

pub(crate) fn list_global_mcp_servers(state: &AppState) -> Result<Vec<GlobalMcpEntry>, String> {
    let key = cache_key(state);
    if let Ok(r) = global_mcp_cache().read() {
        if let Some(cached) = r.get(&key) {
            return Ok(cached.clone());
        }
    }
    let result = with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT name, config_json, is_builtin FROM global_mcp_servers ORDER BY is_builtin DESC, name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GlobalMcpEntry {
                    name: row.get(0)?,
                    config_json: row.get(1)?,
                    is_builtin: row.get::<_, i64>(2)? != 0,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    })?;
    if let Ok(mut w) = global_mcp_cache().write() {
        w.insert(key, result.clone());
    }
    Ok(result)
}

fn validate_mcp_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("MCP server name required".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("MCP server name only allows letters, numbers, - and _".to_string());
    }
    Ok(())
}

pub(crate) fn upsert_global_mcp_server(
    state: &AppState,
    name: &str,
    config_json: &str,
    is_builtin: bool,
) -> Result<(), String> {
    validate_mcp_name(name)?;
    let now = now_ms();
    let result = with_db(state, |conn| {
        conn.execute(
            "INSERT INTO global_mcp_servers (name, config_json, is_builtin, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(name) DO UPDATE SET
               config_json = excluded.config_json,
               updated_at = excluded.updated_at
             WHERE is_builtin = 0",
            params![name, config_json, is_builtin as i64, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
    invalidate_global_mcp_cache(state);
    result
}

pub(crate) fn delete_global_mcp_server(state: &AppState, name: &str) -> Result<(), String> {
    let result = with_db(state, |conn| {
        let deleted = conn
            .execute(
                "DELETE FROM global_mcp_servers WHERE name = ?1 AND is_builtin = 0",
                params![name],
            )
            .map_err(|e| e.to_string())?;
        if deleted == 0 {
            return Err(format!(
                "MCP server '{}' not found or is a built-in server",
                name
            ));
        }
        Ok(())
    });
    invalidate_global_mcp_cache(state);
    result
}

pub(crate) fn load_all_global_mcp_as_acp(state: &AppState) -> Vec<acp::McpServer> {
    let key = cache_key(state);
    if let Ok(r) = global_mcp_acp_cache().read() {
        if let Some(cached) = r.get(&key) {
            return cached.clone();
        }
    }
    let servers: Vec<acp::McpServer> = list_global_mcp_servers(state)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let json = inject_name_if_missing(&entry.config_json, &entry.name);
            parse_mcp_server_json_compat(&json)
        })
        .collect();
    if let Ok(mut w) = global_mcp_acp_cache().write() {
        w.insert(key, servers.clone());
    }
    servers
}

fn inject_name_if_missing(config_json: &str, name: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(config_json) else {
        return config_json.to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        obj.entry("name".to_string())
            .or_insert_with(|| serde_json::Value::String(name.to_string()));
    }
    value.to_string()
}

fn normalize_mcp_server_json(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("command") {
            obj.entry("args".to_string()).or_insert_with(|| json!([]));
            obj.entry("env".to_string()).or_insert_with(|| json!([]));
        }
        if obj.contains_key("url") {
            obj.entry("headers".to_string())
                .or_insert_with(|| json!([]));
        }
    }
    value
}

pub(crate) fn parse_mcp_server_json_compat(raw: &str) -> Option<acp::McpServer> {
    serde_json::from_str::<acp::McpServer>(raw)
        .ok()
        .or_else(|| {
            let value = serde_json::from_str::<Value>(raw).ok()?;
            serde_json::from_value::<acp::McpServer>(normalize_mcp_server_json(value)).ok()
        })
}

pub(crate) fn parse_mcp_server_list_json_compat(raw: &str) -> Vec<acp::McpServer> {
    serde_json::from_str::<Vec<acp::McpServer>>(raw)
        .ok()
        .or_else(|| {
            let values = serde_json::from_str::<Vec<Value>>(raw).ok()?;
            Some(
                values
                    .into_iter()
                    .filter_map(|v| {
                        serde_json::from_value::<acp::McpServer>(v.clone())
                            .ok()
                            .or_else(|| {
                                serde_json::from_value::<acp::McpServer>(normalize_mcp_server_json(
                                    v,
                                ))
                                .ok()
                            })
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::parse_mcp_server_list_json_compat;

    #[test]
    fn parses_legacy_stdio_without_env() {
        let raw = r#"[{"name":"chrome-devtools","command":"npx","args":["-y","@chrome-devtools/chrome-devtools-mcp"]}]"#;
        let parsed = parse_mcp_server_list_json_compat(raw);
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn parses_legacy_http_without_headers() {
        let raw = r#"[{"type":"http","name":"demo","url":"http://127.0.0.1:9999"}]"#;
        let parsed = parse_mcp_server_list_json_compat(raw);
        assert_eq!(parsed.len(), 1);
    }
}

#[tauri::command]
pub(crate) fn list_global_mcp_servers_cmd(
    state: State<'_, AppState>,
) -> Result<Vec<GlobalMcpEntry>, String> {
    list_global_mcp_servers(get_state(&state))
}

#[tauri::command]
pub(crate) fn upsert_global_mcp_server_cmd(
    state: State<'_, AppState>,
    name: String,
    config_json: String,
) -> Result<(), String> {
    upsert_global_mcp_server(get_state(&state), &name, &config_json, false)
}

#[tauri::command]
pub(crate) fn delete_global_mcp_server_cmd(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    delete_global_mcp_server(get_state(&state), &name)
}

pub(crate) fn list_role_mcp_servers(
    state: &AppState,
    role_name: &str,
) -> Result<Vec<RoleMcpEntry>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT g.name, g.config_json, g.is_builtin, COALESCE(r.enabled, 0)
                 FROM global_mcp_servers g
                 LEFT JOIN role_mcp_servers r ON r.mcp_server_name = g.name AND r.role_name = ?1
                 ORDER BY g.is_builtin DESC, g.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| {
                Ok(RoleMcpEntry {
                    mcp_server_name: row.get(0)?,
                    config_json: row.get(1)?,
                    is_builtin: row.get::<_, i64>(2)? != 0,
                    enabled: row.get::<_, i64>(3)? != 0,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    })
}

pub(crate) fn set_role_mcp_enabled(
    state: &AppState,
    role_name: &str,
    mcp_server_name: &str,
    enabled: bool,
) -> Result<(), String> {
    with_db(state, |conn| {
        if enabled {
            conn.execute(
                "INSERT INTO role_mcp_servers (role_name, mcp_server_name, enabled)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(role_name, mcp_server_name) DO UPDATE SET enabled = 1",
                params![role_name, mcp_server_name],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "INSERT INTO role_mcp_servers (role_name, mcp_server_name, enabled)
                 VALUES (?1, ?2, 0)
                 ON CONFLICT(role_name, mcp_server_name) DO UPDATE SET enabled = 0",
                params![role_name, mcp_server_name],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

pub(crate) fn get_enabled_mcp_for_role(state: &AppState, role_name: &str) -> Vec<acp::McpServer> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT g.config_json, g.name
                 FROM global_mcp_servers g
                 JOIN role_mcp_servers r ON r.mcp_server_name = g.name AND r.role_name = ?1
                 WHERE r.enabled = 1
                 ORDER BY g.is_builtin DESC, g.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            if let Ok((cfg, name)) = row {
                let injected = inject_name_if_missing(&cfg, &name);
                if let Some(srv) = parse_mcp_server_json_compat(&injected) {
                    result.push(srv);
                }
            }
        }
        Ok(result)
    })
    .unwrap_or_default()
}

#[tauri::command]
pub(crate) fn list_role_mcp_servers_cmd(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<RoleMcpEntry>, String> {
    list_role_mcp_servers(get_state(&state), &role_name)
}

#[tauri::command]
pub(crate) fn set_role_mcp_enabled_cmd(
    state: State<'_, AppState>,
    role_name: String,
    mcp_server_name: String,
    enabled: bool,
) -> Result<(), String> {
    set_role_mcp_enabled(get_state(&state), &role_name, &mcp_server_name, enabled)
}

pub(crate) fn seed_builtin_jockey_mcp(state: &AppState, port: u16, token: &str) {
    let config = serde_json::json!({
        "type": "http",
        "name": "jockey",
        "url": format!("http://127.0.0.1:{port}"),
        "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}]
    });
    let _ = with_db(state, |conn| {
        let now = now_ms();
        conn.execute(
            "INSERT INTO global_mcp_servers (name, config_json, is_builtin, created_at, updated_at)
             VALUES ('jockey', ?1, 1, ?2, ?2)
             ON CONFLICT(name) DO UPDATE SET config_json = excluded.config_json, updated_at = excluded.updated_at",
            params![config.to_string(), now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
    invalidate_global_mcp_cache(state);
}
