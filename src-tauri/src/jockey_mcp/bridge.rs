use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::db::app_session::{close_app_session_internal, create_app_session_internal};
use crate::db::context::{
    clear_shared_context_internal, list_shared_context_internal, set_shared_context_internal,
};
use crate::db::global_mcp::{
    delete_global_mcp_server, list_global_mcp_servers, upsert_global_mcp_server,
};
use crate::db::role::{delete_role_internal, list_all_roles, load_role, upsert_role};
use crate::db::skill::{
    delete_skill_internal, list_skills_internal, load_skill_by_name, upsert_skill_internal,
};
use crate::db::with_db;
use crate::db::workflow::{
    create_workflow_internal, delete_workflow_internal, list_workflows_internal, load_workflow,
    update_workflow_internal,
};
use crate::types::{AppSkillUpsert, AppState};

const MAX_BODY_SIZE: usize = 1024 * 1024;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

static BRIDGE_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
static BRIDGE_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static BRIDGE_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn bridge_port() -> Option<u16> {
    BRIDGE_PORT.get().copied()
}

pub fn bridge_error() -> Option<&'static str> {
    BRIDGE_ERROR.get().map(|s| s.as_str())
}

pub fn bridge_token() -> Option<&'static str> {
    BRIDGE_TOKEN.get().map(|s| s.as_str())
}

pub(crate) async fn start_bridge(state: Arc<AppState>) -> Result<(u16, String), String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| {
        let msg = format!("bridge bind: {e}");
        let _ = BRIDGE_ERROR.set(msg.clone());
        msg
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| {
            let msg = format!("bridge addr: {e}");
            let _ = BRIDGE_ERROR.set(msg.clone());
            msg
        })?
        .port();
    let _ = BRIDGE_PORT.set(port);

    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let _ = BRIDGE_TOKEN.set(token.clone());
    let token_arc = std::sync::Arc::new(token.clone());

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let state = state.clone();
            let tok = token_arc.clone();
            tokio::spawn(async move {
                let result =
                    tokio::time::timeout(REQUEST_TIMEOUT, handle_connection(stream, &state, &tok))
                        .await;
                if result.is_err() {
                    eprintln!("[jockey-mcp] request timed out");
                }
            });
        }
    });

    Ok((port, token))
}

async fn handle_connection(stream: tokio::net::TcpStream, state: &AppState, token: &str) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.is_err() || request_line.is_empty() {
        return;
    }
    let is_post = request_line.starts_with("POST ");

    let mut content_length: usize = 0;
    let mut authorized = false;
    let expected_header = format!("Bearer {token}");
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            _ => {}
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(val) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = val.trim().parse().unwrap_or(0);
        }
        if let Some(val) = line
            .strip_prefix("Authorization:")
            .or_else(|| line.strip_prefix("authorization:"))
        {
            if val.trim() == expected_header {
                authorized = true;
            }
        }
    }

    if !authorized {
        let resp = http_response(401, r#"{"error":"unauthorized"}"#);
        let _ = writer.write_all(resp.as_bytes()).await;
        return;
    }

    if !is_post {
        let resp = http_response(405, r#"{"error":"method not allowed"}"#);
        let _ = writer.write_all(resp.as_bytes()).await;
        return;
    }

    if content_length > MAX_BODY_SIZE {
        let resp = http_response(413, r#"{"error":"request too large"}"#);
        let _ = writer.write_all(resp.as_bytes()).await;
        return;
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        if reader.read_exact(&mut body).await.is_err() {
            let resp = http_response(400, r#"{"error":"incomplete body"}"#);
            let _ = writer.write_all(resp.as_bytes()).await;
            return;
        }
    }
    let body_str = match String::from_utf8(body) {
        Ok(s) => s,
        Err(_) => {
            let resp = http_response(400, r#"{"error":"invalid utf-8 body"}"#);
            let _ = writer.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let response_body = handle_mcp_request(state, &body_str);
    let resp = http_response(200, &response_body);
    let _ = writer.write_all(resp.as_bytes()).await;
}

fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn handle_mcp_request(state: &AppState, body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Req {
        id: Option<Value>,
        method: String,
        #[serde(default)]
        params: Value,
    }

    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            }))
            .unwrap_or_default();
        }
    };

    let id = req.id.clone().unwrap_or(Value::Null);

    let resp = match req.method.as_str() {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "jockey", "version": "0.1.0" }
            }
        }),
        "notifications/initialized" => return String::new(),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tool_definitions() }
        }),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));
            match dispatch(state, name, arguments) {
                Ok(result) => {
                    let text = match result {
                        Value::String(s) => s,
                        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
                    };
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": [{ "type": "text", "text": text }] }
                    })
                }
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true }
                }),
            }
        }
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {}", req.method) }
        }),
    };

    serde_json::to_string(&resp).unwrap_or_default()
}

pub(super) fn dispatch(state: &AppState, method: &str, params: Value) -> Result<Value, String> {
    match method {
        // ── Roles ──────────────────────────────────────────────────────────
        "list_roles" => {
            let roles = list_all_roles(state)?;
            let out: Vec<Value> = roles
                .iter()
                .map(|r| {
                    json!({
                        "roleName": r.role_name,
                        "runtimeKind": r.runtime_kind,
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
        "get_role" => {
            let role_name = params
                .get("roleName")
                .and_then(|v| v.as_str())
                .ok_or("roleName is required")?;
            let role = load_role(state, role_name)?
                .ok_or_else(|| format!("role not found: {role_name}"))?;
            Ok(json!({
                "roleName": role.role_name,
                "runtimeKind": role.runtime_kind,
                "systemPrompt": role.system_prompt,
                "model": role.model,
                "mode": role.mode,
                "mcpServers": serde_json::from_str::<Value>(&role.mcp_servers_json).unwrap_or(json!([])),
                "configOptions": serde_json::from_str::<Value>(&role.config_options_json).unwrap_or(json!({})),
                "configOptionDefs": serde_json::from_str::<Value>(&role.config_option_defs_json).unwrap_or(json!([])),
                "autoApprove": role.auto_approve,
            }))
        }
        "upsert_role" => {
            let role_name = params
                .get("roleName")
                .and_then(|v| v.as_str())
                .ok_or("roleName is required")?
                .to_string();
            let runtime_kind = params
                .get("runtimeKind")
                .and_then(|v| v.as_str())
                .unwrap_or("claude-code")
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

            upsert_role(
                state,
                role_name.clone(),
                runtime_kind.clone(),
                system_prompt,
                model,
                mode,
                mcp_servers_json,
                config_options_json,
                config_option_defs_json,
                auto_approve,
            )?;
            Ok(json!(format!(
                "Role '{role_name}' saved (runtime: {runtime_kind})"
            )))
        }
        "delete_role" => {
            let role_name = params
                .get("roleName")
                .and_then(|v| v.as_str())
                .ok_or("roleName is required")?;
            delete_role_internal(state, role_name)?;
            Ok(json!(format!("Role '{role_name}' deleted")))
        }
        // ── MCP (role-level) ───────────────────────────────────────────────
        "list_mcp_servers" => {
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
                let role = roles
                    .iter()
                    .find(|r| r.role_name == rn)
                    .ok_or_else(|| format!("role not found: {rn}"))?;
                let role_servers: Value =
                    serde_json::from_str(&role.mcp_servers_json).unwrap_or(json!([]));
                return Ok(json!({
                    "globalRegistry": global_out,
                    "roleServers": role_servers,
                }));
            }

            let roles_out: Vec<Value> = roles
                .iter()
                .map(|r| {
                    json!({
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
        "add_mcp_to_role" => {
            let role_name = params
                .get("roleName")
                .and_then(|v| v.as_str())
                .ok_or("roleName is required")?;
            let server = params.get("server").ok_or("server object is required")?;
            let new_name = server.get("name").and_then(|v| v.as_str()).unwrap_or("");

            let role = load_role(state, role_name)?
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
            )?;

            Ok(json!(format!(
                "Added MCP server '{new_name}' to role '{role_name}'. Total servers: {}",
                servers.len()
            )))
        }
        "remove_mcp_from_role" => {
            let role_name = params
                .get("roleName")
                .and_then(|v| v.as_str())
                .ok_or("roleName is required")?;
            let server_name = params
                .get("serverName")
                .and_then(|v| v.as_str())
                .ok_or("serverName is required")?;

            let role = load_role(state, role_name)?
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
            )?;

            Ok(json!(format!(
                "Removed MCP server '{server_name}' from role '{role_name}'. Remaining: {}",
                servers.len()
            )))
        }
        // ── MCP (global registry) ──────────────────────────────────────────
        "upsert_global_mcp" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?;
            let config = params.get("config").ok_or("config object is required")?;
            let config_json = serde_json::to_string(config).map_err(|e| e.to_string())?;
            upsert_global_mcp_server(state, name, &config_json, false)?;
            Ok(json!(format!("Global MCP server '{name}' saved")))
        }
        "delete_global_mcp" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?;
            delete_global_mcp_server(state, name)?;
            Ok(json!(format!("Global MCP server '{name}' deleted")))
        }
        // ── Skills ─────────────────────────────────────────────────────────
        "list_skills" => {
            let skills = list_skills_internal(state)?;
            let out: Vec<Value> = skills
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "description": s.description,
                    })
                })
                .collect();
            Ok(json!(out))
        }
        "get_skill" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?;
            let skill = load_skill_by_name(state, name)?
                .ok_or_else(|| format!("skill not found: {name}"))?;
            Ok(json!({
                "id": skill.id,
                "name": skill.name,
                "description": skill.description,
                "content": skill.content,
            }))
        }
        "upsert_skill" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?
                .to_string();
            let description = params
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let skill = upsert_skill_internal(
                state,
                AppSkillUpsert {
                    id,
                    name: name.clone(),
                    description,
                    content,
                },
            )?;
            Ok(json!(format!(
                "Skill '{}' saved (id: {})",
                skill.name, skill.id
            )))
        }
        "delete_skill" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            delete_skill_internal(state, id)?;
            Ok(json!(format!("Skill '{id}' deleted")))
        }
        // ── Sessions ───────────────────────────────────────────────────────
        "list_sessions" => {
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let sessions = with_db(state, |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, title, active_role, runtime_kind, cwd, created_at, last_active_at \
                         FROM app_sessions WHERE closed_at IS NULL ORDER BY last_active_at DESC LIMIT ?1",
                    )
                    .map_err(|e| e.to_string())?;
                let rows: Vec<Value> = stmt
                    .query_map(rusqlite::params![limit], |row| {
                        Ok(json!({
                            "id": row.get::<_, String>(0)?,
                            "title": row.get::<_, Option<String>>(1)?,
                            "activeRole": row.get::<_, Option<String>>(2)?,
                            "runtimeKind": row.get::<_, Option<String>>(3)?,
                            "cwd": row.get::<_, Option<String>>(4)?,
                            "createdAt": row.get::<_, i64>(5)?,
                            "lastActiveAt": row.get::<_, i64>(6)?,
                        }))
                    })
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                Ok(rows)
            })?;
            Ok(json!(sessions))
        }
        "get_session" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let session = with_db(state, |conn| {
                conn.query_row(
                    "SELECT id, title, active_role, runtime_kind, cwd, created_at, last_active_at \
                     FROM app_sessions WHERE id = ?1",
                    rusqlite::params![id],
                    |row| {
                        Ok(json!({
                            "id": row.get::<_, String>(0)?,
                            "title": row.get::<_, Option<String>>(1)?,
                            "activeRole": row.get::<_, Option<String>>(2)?,
                            "runtimeKind": row.get::<_, Option<String>>(3)?,
                            "cwd": row.get::<_, Option<String>>(4)?,
                            "createdAt": row.get::<_, i64>(5)?,
                            "lastActiveAt": row.get::<_, i64>(6)?,
                        }))
                    },
                )
                .map_err(|e| e.to_string())
            })?;
            Ok(session)
        }
        "update_session" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let now = crate::now_ms();
            with_db(state, |conn| {
                if let Some(title) = params.get("title").and_then(|v| v.as_str()) {
                    conn.execute(
                        "UPDATE app_sessions SET title = ?1, last_active_at = ?2 WHERE id = ?3",
                        rusqlite::params![title, now, id],
                    )
                    .map_err(|e| e.to_string())?;
                }
                if let Some(role) = params.get("activeRole").and_then(|v| v.as_str()) {
                    conn.execute(
                        "UPDATE app_sessions SET active_role = ?1, last_active_at = ?2 WHERE id = ?3",
                        rusqlite::params![role, now, id],
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(())
            })?;
            Ok(json!(format!("Session '{id}' updated")))
        }
        "create_session" => {
            let title = params.get("title").and_then(|v| v.as_str());
            let session = create_app_session_internal(state, title)?;
            Ok(json!({
                "id": session.id,
                "title": session.title,
                "activeRole": session.active_role,
            }))
        }
        "close_session" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            close_app_session_internal(state, id)?;
            Ok(json!(format!("Session '{id}' closed")))
        }
        "get_session_history" => {
            let session_id = params
                .get("sessionId")
                .and_then(|v| v.as_str())
                .ok_or("sessionId is required")?;
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let messages = with_db(state, |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT role_name, content, created_at FROM app_session_messages \
                         WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
                    )
                    .map_err(|e| e.to_string())?;
                let rows: Vec<Value> = stmt
                    .query_map(rusqlite::params![session_id, limit], |row| {
                        Ok(json!({
                            "roleName": row.get::<_, String>(0)?,
                            "content": row.get::<_, String>(1)?,
                            "createdAt": row.get::<_, i64>(2)?,
                        }))
                    })
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                Ok(rows)
            })?;
            let mut messages = messages;
            messages.reverse();
            Ok(json!(messages))
        }
        // ── Workflows ──────────────────────────────────────────────────────
        "get_workflow" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let wf = load_workflow(state, id)?;
            Ok(json!({
                "id": wf.id,
                "name": wf.name,
                "steps": wf.steps,
                "createdAt": wf.created_at,
                "updatedAt": wf.updated_at,
            }))
        }
        "update_workflow" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let steps: Option<Vec<String>> =
                params.get("steps").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(|x| x.to_string()))
                        .collect()
                });
            let wf = update_workflow_internal(state, id, name, steps)?;
            Ok(json!(format!("Workflow '{}' updated", wf.name)))
        }
        "list_workflows" => {
            let workflows = list_workflows_internal(state)?;
            let out: Vec<Value> = workflows
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id,
                        "name": w.name,
                        "steps": w.steps,
                    })
                })
                .collect();
            Ok(json!(out))
        }
        "create_workflow" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name is required")?
                .to_string();
            let steps: Vec<String> = params
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(|x| x.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let wf = create_workflow_internal(state, name, steps)?;
            Ok(json!(format!(
                "Workflow '{}' created (id: {})",
                wf.name, wf.id
            )))
        }
        "delete_workflow" => {
            let id = params
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id is required")?;
            delete_workflow_internal(state, id)?;
            Ok(json!(format!("Workflow '{id}' deleted")))
        }
        // ── Shared context ─────────────────────────────────────────────────
        "set_shared_context" => {
            let scope = params
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("global");
            let key = params
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("key is required")?;
            let value = params
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("value is required")?;
            set_shared_context_internal(state, scope, key, value)?;
            Ok(json!(format!("Context set: [{scope}] {key}")))
        }
        "get_shared_context" => {
            let scope = params
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("global");
            let entries = list_shared_context_internal(state, scope)?;
            let out: Vec<Value> = entries
                .iter()
                .map(|e| json!({ "key": e.key, "value": e.value }))
                .collect();
            Ok(json!(out))
        }
        "delete_shared_context" => {
            let scope = params
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("global");
            let key = params
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("key is required")?;
            clear_shared_context_internal(state, scope, key)?;
            Ok(json!(format!("Context deleted: [{scope}] {key}")))
        }
        _ => Err(format!("unknown method: {method}")),
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        // Roles
        json!({ "name": "list_roles", "description": "List all configured roles. A role is a named persona that wraps a runtime (claude-code, codex-cli, gemini-cli) with a system prompt, model override, mode, and MCP servers. You (the agent) are running inside one of these roles right now — call get_context to find out which one.", "inputSchema": { "type": "object", "properties": {} } }),
        json!({
            "name": "get_role", "description": "Get full details of a role including system prompt, MCP servers, config option definitions, and saved option values.",
            "inputSchema": {
                "type": "object",
                "properties": { "roleName": { "type": "string", "description": "Role name. Call list_roles to get available names." } },
                "required": ["roleName"]
            }
        }),
        json!({
            "name": "upsert_role", "description": "Create or update a role. Omitted fields preserve existing values.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Role name (unique identifier)." },
                    "runtimeKind": { "type": "string", "description": "Runtime to use: claude-code, gemini-cli, or codex-cli.", "default": "claude-code" },
                    "systemPrompt": { "type": "string", "description": "System prompt injected at the start of each session." },
                    "model": { "type": "string", "description": "Model override (e.g. claude-opus-4-5). Leave empty to use the runtime default." },
                    "mode": { "type": "string", "description": "Execution mode (e.g. acceptEdits). Leave empty for the runtime default." },
                    "autoApprove": { "type": "boolean", "description": "If true, tool calls are auto-approved without user confirmation." },
                    "mcpServersJson": { "type": "string", "description": "JSON array of MCP server objects to set, replacing any existing servers. Omit to preserve current servers." },
                    "configOptionsJson": { "type": "string", "description": "JSON object of saved option values by option id, for example {\"model\":\"claude-sonnet-4\"}. Omit to preserve current values." },
                    "configOptionDefsJson": { "type": "string", "description": "JSON array of config option definitions discovered from runtime prewarm. Omit to preserve current definitions." }
                },
                "required": ["roleName"]
            }
        }),
        json!({
            "name": "delete_role", "description": "Delete a role by name.",
            "inputSchema": {
                "type": "object",
                "properties": { "roleName": { "type": "string", "description": "Role name to delete." } },
                "required": ["roleName"]
            }
        }),
        // MCP role-level
        json!({
            "name": "list_mcp_servers",
            "description": "List MCP servers configured in Jockey. Returns the global registry (servers available to all roles) and per-role assignments. The 'jockey' server is always injected automatically — it is this tool server itself. MCP servers attached to a role are passed into the agent's session at startup; after adding/removing a server the role must be reconnected for the change to take effect. Provide roleName to scope results to one role.",
            "inputSchema": {
                "type": "object",
                "properties": { "roleName": { "type": "string", "description": "Optional. Filter to a single role. Call list_roles for available names." } }
            }
        }),
        json!({
            "name": "add_mcp_to_role",
            "description": "Add an MCP server to a role. For stdio servers provide command+args+env. For HTTP/SSE servers provide type+url+headers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Role to add the server to." },
                    "server": {
                        "type": "object",
                        "description": "MCP server definition.",
                        "properties": {
                            "name": { "type": "string", "description": "Unique name for this server on the role." },
                            "command": { "type": "string", "description": "Executable to launch (stdio transport only)." },
                            "args": { "type": "array", "items": { "type": "string" }, "description": "CLI arguments passed to command (stdio transport)." },
                            "env": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "value": { "type": "string" } } }, "description": "Environment variables for the subprocess (stdio transport)." },
                            "type": { "type": "string", "enum": ["http", "sse"], "description": "Transport type for remote servers. Omit for stdio." },
                            "url": { "type": "string", "description": "Server URL (http or sse transport)." },
                            "headers": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "value": { "type": "string" } } }, "description": "HTTP headers sent with each request (http/sse transport)." }
                        },
                        "required": ["name"]
                    }
                },
                "required": ["roleName", "server"]
            }
        }),
        json!({
            "name": "remove_mcp_from_role",
            "description": "Remove an MCP server from a role by server name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "description": "Role to remove the server from." },
                    "serverName": { "type": "string", "description": "Name of the MCP server to remove." }
                },
                "required": ["roleName", "serverName"]
            }
        }),
        // MCP global registry
        json!({
            "name": "upsert_global_mcp",
            "description": "Register or update an MCP server in the global registry. Use add_mcp_to_role to attach it to specific roles.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique server name in the global registry." },
                    "config": {
                        "type": "object",
                        "description": "Server config. stdio: {command, args[], env[]}. Remote: {type:'http'|'sse', url, headers[]}."
                    }
                },
                "required": ["name", "config"]
            }
        }),
        json!({
            "name": "delete_global_mcp",
            "description": "Remove a custom MCP server from the global registry. Built-in servers cannot be deleted.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Server name to delete from the global registry." } },
                "required": ["name"]
            }
        }),
        // Skills
        json!({ "name": "list_skills", "description": "List all skills with id, name, and description.", "inputSchema": { "type": "object", "properties": {} } }),
        json!({
            "name": "get_skill", "description": "Get full content of a skill by name.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Skill name. Call list_skills for available names." } },
                "required": ["name"]
            }
        }),
        json!({
            "name": "upsert_skill", "description": "Create or update a skill. Provide id to update an existing skill; omit id to create a new one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Skill id. Omit to create a new skill; provide to update an existing one." },
                    "name": { "type": "string", "description": "Skill name (unique identifier shown in /skill completions)." },
                    "description": { "type": "string", "description": "Short description shown to agents when selecting skills." },
                    "content": { "type": "string", "description": "Full skill prompt/instructions content." }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "delete_skill", "description": "Delete a skill by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Skill id to delete. Call list_skills to get available ids." } },
                "required": ["id"]
            }
        }),
        // Sessions
        json!({
            "name": "list_sessions", "description": "List active (non-closed) sessions ordered by most recently active.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum number of sessions to return. Default is 50.", "default": 50 }
                }
            }
        }),
        json!({
            "name": "get_session", "description": "Get details of a session by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Session id. Call list_sessions to get available ids." } },
                "required": ["id"]
            }
        }),
        json!({
            "name": "update_session", "description": "Update a session's title or active role.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Session id." },
                    "title": { "type": "string", "description": "New title (no spaces)." },
                    "activeRole": { "type": "string", "description": "Role name to set as the active role." }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "create_session", "description": "Create a new chat session.",
            "inputSchema": {
                "type": "object",
                "properties": { "title": { "type": "string", "description": "Session title (no spaces). Defaults to Session_1." } }
            }
        }),
        json!({
            "name": "close_session", "description": "Close (soft-delete) a session by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Session id to close." } },
                "required": ["id"]
            }
        }),
        json!({
            "name": "get_session_history",
            "description": "Get recent chat messages for a session, ordered chronologically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string", "description": "Session id. Call list_sessions to get available ids." },
                    "limit": { "type": "integer", "description": "Maximum number of messages to return. Default is 50.", "default": 50 }
                },
                "required": ["sessionId"]
            }
        }),
        // Workflows
        json!({ "name": "list_workflows", "description": "List all workflows.", "inputSchema": { "type": "object", "properties": {} } }),
        json!({
            "name": "get_workflow", "description": "Get full details of a workflow by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Workflow id. Call list_workflows to get available ids." } },
                "required": ["id"]
            }
        }),
        json!({
            "name": "create_workflow", "description": "Create a workflow with an ordered list of role steps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Workflow name (no spaces)." },
                    "steps": { "type": "array", "items": { "type": "string" }, "description": "Ordered list of role names to execute in sequence." }
                },
                "required": ["name", "steps"]
            }
        }),
        json!({
            "name": "update_workflow", "description": "Update a workflow's name or steps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Workflow id." },
                    "name": { "type": "string", "description": "New workflow name (no spaces)." },
                    "steps": { "type": "array", "items": { "type": "string" }, "description": "Replacement ordered list of role names." }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "delete_workflow", "description": "Delete a workflow by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Workflow id to delete." } },
                "required": ["id"]
            }
        }),
        // Shared context
        json!({
            "name": "set_shared_context", "description": "Set a key-value entry in shared context. Use scopes to namespace entries (e.g. a session id or 'global').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "description": "Namespace for the entry. Default is 'global'.", "default": "global" },
                    "key": { "type": "string", "description": "Context key." },
                    "value": { "type": "string", "description": "Context value." }
                },
                "required": ["key", "value"]
            }
        }),
        json!({
            "name": "get_shared_context", "description": "Get all shared context entries for a scope.",
            "inputSchema": {
                "type": "object",
                "properties": { "scope": { "type": "string", "description": "Scope to query. Default is 'global'.", "default": "global" } }
            }
        }),
        json!({
            "name": "delete_shared_context", "description": "Delete a shared context entry by scope and key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "description": "Scope of the entry. Default is 'global'.", "default": "global" },
                    "key": { "type": "string", "description": "Key to delete." }
                },
                "required": ["key"]
            }
        }),
    ]
}
