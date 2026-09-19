use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use super::handlers::{context, roles, sessions, skills, workflows};
use crate::types::AppState;

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

pub(crate) async fn start_bridge(
    state: Arc<AppState>,
    app: tauri::AppHandle,
) -> Result<(u16, String), String> {
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
            let app = app.clone();
            let tok = token_arc.clone();
            tokio::spawn(async move {
                let result = tokio::time::timeout(
                    REQUEST_TIMEOUT,
                    handle_connection(stream, &state, &app, &tok),
                )
                .await;
                if result.is_err() {
                    eprintln!("[jockey-mcp] request timed out");
                }
            });
        }
    });

    Ok((port, token))
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: &AppState,
    app: &tauri::AppHandle,
    token: &str,
) {
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
    if content_length > 0 && reader.read_exact(&mut body).await.is_err() {
        let resp = http_response(400, r#"{"error":"incomplete body"}"#);
        let _ = writer.write_all(resp.as_bytes()).await;
        return;
    }
    let body_str = match String::from_utf8(body) {
        Ok(s) => s,
        Err(_) => {
            let resp = http_response(400, r#"{"error":"invalid utf-8 body"}"#);
            let _ = writer.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let response_body = handle_mcp_request(state, app, &body_str).await;
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

async fn handle_mcp_request(state: &AppState, app: &tauri::AppHandle, body: &str) -> String {
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
            match dispatch(state, app, name, arguments).await {
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

async fn dispatch(
    state: &AppState,
    app: &tauri::AppHandle,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    match method {
        // Roles
        "list_roles" => roles::list_roles(state),
        "get_role" => roles::get_role(state, params),
        "upsert_role" => roles::upsert_role_handler(state, params),
        "delete_role" => roles::delete_role_handler(state, params),
        "invoke_role" => roles::invoke_role(state, app, params).await,
        // MCP (role-level + global registry)
        "list_mcp_servers" => roles::list_mcp_servers(state, params),
        "add_mcp_to_role" => roles::add_mcp_to_role(state, params),
        "remove_mcp_from_role" => roles::remove_mcp_from_role(state, params),
        "upsert_global_mcp" => roles::upsert_global_mcp(state, params),
        "delete_global_mcp" => roles::delete_global_mcp(state, params),
        // Skills
        "list_skills" => skills::list_skills(state),
        "get_skill" => skills::get_skill(state, params),
        "upsert_skill" => skills::upsert_skill(state, params),
        "delete_skill" => skills::delete_skill(state, params),
        // Sessions
        "list_sessions" => sessions::list_sessions(state, params),
        "get_session" => sessions::get_session(state, params),
        "update_session" => sessions::update_session(state, params),
        "create_session" => sessions::create_session(state, params),
        "close_session" => sessions::close_session(state, params),
        "get_session_context" => sessions::get_session_context(state, params),
        "get_session_history" => sessions::get_session_history(state, params),
        // Workflows
        "get_workflow" => workflows::get_workflow(state, params),
        "update_workflow" => workflows::update_workflow(state, params),
        "list_workflows" => workflows::list_workflows(state),
        "create_workflow" => workflows::create_workflow(state, params),
        "delete_workflow" => workflows::delete_workflow(state, params),
        // Shared context
        "set_shared_context" => context::set_shared_context(state, params),
        "get_shared_context" => context::get_shared_context(state, params),
        "delete_shared_context" => context::delete_shared_context(state, params),
        "get_role_context" => context::get_role_context(state, params),
        _ => Err(format!("unknown method: {method}")),
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "get_session_context",
            "description": "Read the current Jockey session without receiving a proactively injected transcript. Use summary for a compact overview, roles for role activity, turns for grouped conversation turns, or messages for precise message filtering.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "appSessionId": { "type": "string", "description": "Jockey app session id from the Jockey context note." },
                    "view": { "type": "string", "enum": ["summary", "roles", "turns", "messages"], "default": "summary" },
                    "roleName": { "type": "string", "description": "Optional exact role filter for messages and turns." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 5, "description": "Maximum messages or turns to inspect." },
                    "order": { "type": "string", "enum": ["latest", "oldest"], "default": "latest" },
                    "cursor": { "type": "integer", "description": "Message id returned as nextCursor for the next page." },
                    "messageTypes": { "type": "array", "items": { "type": "string", "enum": ["user", "assistant", "tool", "thought", "event"] }, "description": "Optional semantic message filters." },
                    "include": { "type": "array", "items": { "type": "string", "enum": ["text", "toolSummary", "tools", "toolOutput", "raw"] }, "description": "Optional fields to include. Defaults to text and toolSummary." },
                    "includePayload": { "type": "boolean", "default": false, "description": "Compatibility shortcut for include:[raw]." }
                },
                "required": ["appSessionId"]
            }
        }),
        json!({
            "name": "list_roles",
            "description": "List configured roles and their runtime identity. Use get_session_context to inspect this session's conversation instead of receiving history automatically.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_skills",
            "description": "List available skills with their ids, names, and descriptions.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_skill",
            "description": "Read the full content of a skill by name when the task explicitly needs it.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "minLength": 1, "maxLength": 128 } },
                "required": ["name"]
            }
        }),
        json!({
            "name": "invoke_role",
            "description": "Explicitly delegate a bounded sub-task to another configured role in this Jockey session. The target role can inspect the same session through get_session_context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roleName": { "type": "string", "minLength": 1, "maxLength": 128 },
                    "prompt": { "type": "string", "minLength": 1, "maxLength": 32000 },
                    "appSessionId": { "type": "string", "minLength": 1, "description": "Jockey app session id for workspace and transcript ownership." }
                },
                "required": ["roleName", "prompt", "appSessionId"]
            }
        }),
    ]
}
