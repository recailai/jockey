use agent_client_protocol as acp;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::db::{get_state, with_db};
use crate::types::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalMcpEntry {
    pub name: String,
    pub config_json: String,
    pub is_builtin: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn list_global_mcp_servers(state: &AppState) -> Result<Vec<GlobalMcpEntry>, String> {
    with_db(state, |conn| {
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
    })
}

fn validate_mcp_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("MCP server name required".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "MCP server name only allows letters, numbers, - and _".to_string(),
        );
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
    with_db(state, |conn| {
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
    })
}

pub(crate) fn delete_global_mcp_server(state: &AppState, name: &str) -> Result<(), String> {
    with_db(state, |conn| {
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
    })
}

pub(crate) fn load_all_global_mcp_as_acp(state: &AppState) -> Vec<acp::McpServer> {
    list_global_mcp_servers(state)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let json = inject_name_if_missing(&entry.config_json, &entry.name);
            parse_mcp_server_json_compat(&json)
        })
        .collect()
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

pub(crate) fn seed_builtin_jockey_mcp(state: &AppState, port: u16) {
    let config = serde_json::json!({
        "type": "http",
        "name": "jockey",
        "url": format!("http://127.0.0.1:{port}"),
        "headers": []
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
}
