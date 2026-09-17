//! Native Claude permission bridge.
//!
//! The native Claude CLI exposes per-tool-use authorization through
//! `--permission-prompt-tool mcp__jockey_perm__request_permission`. Jockey
//! satisfies that contract without shipping a second binary:
//!
//! 1. At app startup, [`start_permission_bridge`] binds a loopback TCP
//!    listener with a random port + per-run secret token.
//! 2. When a permission-gated native Claude session spawns, the headless
//!    runner writes a temp `--mcp-config` file pointing at
//!    `current_exe --__jockey-permission-bridge` and passes
//!    `--permission-prompt-tool` (see [`claude_permission_args`]).
//! 3. Claude spawns the bridge subprocess, which speaks MCP stdio and
//!    forwards each permission request over the loopback socket.
//! 4. The app side registers a `PendingPermission` in the shared permission
//!    map, emits the standard `permissionRequest` stream event (reusing the
//!    existing Jockey permission UI), and blocks the bridge until the user
//!    answers, the request times out, or the bridge disconnects.
//!
//! Denial is the default in every failure path; the runner never falls back
//! to `--dangerously-skip-permissions` unless the role explicitly opts into
//! auto-approve.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

use super::super::adapter::{acp_log, clip};
use super::super::protocol as acp;
use super::super::worker::{
    cancel_permissions_for, insert_permission, permission_requests, AcpEvent, PendingPermission,
};
use super::execute::AcpStreamPayload;

/// How long a permission request may sit unanswered before it is denied.
/// The bridge subprocess uses a slightly larger timeout so the app side
/// always decides first.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(300);
const BRIDGE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const BRIDGE_TIMEOUT_PAD: Duration = Duration::from_secs(30);

/// MCP tool name Claude is instructed to call for permission prompts.
pub(crate) const PERMISSION_TOOL_NAME: &str = "mcp__jockey_perm__request_permission";
/// Hidden subcommand that turns this binary into the bridge's stdio MCP server.
pub(crate) const BRIDGE_SUBCOMMAND: &str = "--__jockey-permission-bridge";

static BRIDGE_PORT: OnceLock<u16> = OnceLock::new();
static BRIDGE_TOKEN: OnceLock<String> = OnceLock::new();
static BRIDGE_READY: AtomicBool = AtomicBool::new(false);
static BRIDGE_SEQ: AtomicU32 = AtomicU32::new(1);

// ── App side ─────────────────────────────────────────────────────────────────

/// Bind the loopback listener and start the accept loop. Called once during
/// app setup; failures only disable native-Claude permission gating (the
/// runner then reports the session as unavailable instead of bypassing).
pub(crate) fn start_permission_bridge(app: tauri::AppHandle) {
    let std_listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(error) => {
            acp_log(
                "perm_bridge.listen.failed",
                json!({ "error": error.to_string() }),
            );
            return;
        }
    };
    let port = match std_listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(error) => {
            acp_log(
                "perm_bridge.listen.failed",
                json!({ "error": error.to_string() }),
            );
            return;
        }
    };
    if let Err(error) = std_listener.set_nonblocking(true) {
        acp_log(
            "perm_bridge.listen.failed",
            json!({ "error": error.to_string() }),
        );
        return;
    }
    if BRIDGE_PORT.set(port).is_err() || BRIDGE_TOKEN.set(uuid::Uuid::new_v4().to_string()).is_err()
    {
        return;
    }
    acp_log("perm_bridge.listen", json!({ "port": port }));
    BRIDGE_READY.store(true, Ordering::Release);
    // The listener is bound with a plain std socket in app setup (a no-op for
    // the tokio runtime), then handed to a dedicated thread that owns a
    // current-thread tokio runtime: `TcpListener::from_std` must be executed
    // inside a runtime context, and app setup runs on the main thread where
    // registering a blocking fd is unsupported (tokio issue #7172).
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                BRIDGE_READY.store(false, Ordering::Release);
                acp_log(
                    "perm_bridge.runtime.failed",
                    json!({ "error": error.to_string() }),
                );
                return;
            }
        };
        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(listener) => listener,
                Err(error) => {
                    BRIDGE_READY.store(false, Ordering::Release);
                    acp_log(
                        "perm_bridge.listen.failed",
                        json!({ "error": error.to_string() }),
                    );
                    return;
                }
            };
            accept_loop(listener, app).await;
        });
    });
}

pub(crate) fn is_ready() -> bool {
    BRIDGE_READY.load(Ordering::Acquire)
}

async fn accept_loop(listener: tokio::net::TcpListener, app: tauri::AppHandle) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let app = app.clone();
                tokio::spawn(handle_bridge_connection(stream, app));
            }
            Err(error) => {
                acp_log(
                    "perm_bridge.accept.failed",
                    json!({ "error": error.to_string() }),
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

async fn handle_bridge_connection(stream: tokio::net::TcpStream, app: tauri::AppHandle) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    // First frame authenticates and names the headless session key.
    let auth_line = match tokio::time::timeout(BRIDGE_CONNECT_TIMEOUT, lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        _ => return,
    };
    let auth: Value = match serde_json::from_str(&auth_line) {
        Ok(value) => value,
        Err(_) => return,
    };
    if auth.get("token").and_then(Value::as_str)
        != Some(BRIDGE_TOKEN.get().map(String::as_str).unwrap_or(""))
    {
        return;
    }
    let Some(key) = auth.get("key").and_then(Value::as_str) else {
        return;
    };
    let Some((app_session_id, runtime_key, role_name)) = parse_headless_key(key) else {
        return;
    };
    acp_log(
        "perm_bridge.connected",
        json!({ "runtime": runtime_key, "role": role_name, "appSessionId": app_session_id }),
    );

    // Request ids this connection is responsible for; denied on disconnect.
    let mut registered: Vec<String> = Vec::new();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            _ => break,
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(request_ref) = request
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
        else {
            continue;
        };
        let tool_name = request
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let input = request.get("input").cloned().unwrap_or_else(|| json!({}));

        let (tx, rx) = oneshot::channel();
        let request_id = uuid::Uuid::new_v4().to_string();
        insert_permission(
            request_id.clone(),
            PendingPermission {
                runtime_key: runtime_key.clone(),
                role_name: role_name.clone(),
                app_session_id: app_session_id.clone(),
                cache_key: String::new(),
                allow_always_option_ids: Vec::new(),
                delta_tx: None,
                tx,
            },
        );
        registered.push(request_id.clone());
        emit_permission_event(
            &app,
            &runtime_key,
            &role_name,
            &app_session_id,
            AcpEvent::PermissionRequest {
                request_id: request_id.clone(),
                title: tool_name.clone(),
                description: Some(clip(&input.to_string(), 500)),
                // The permission modal renders one button per option plus its
                // own destructive Deny button (which responds with
                // `cancelled`); a single allow option is all we need here.
                options: vec![json!({
                    "optionId": "allow",
                    "title": "Allow",
                    "kind": "allow_once",
                })],
            },
        );

        let outcome = tokio::time::timeout(PERMISSION_TIMEOUT, rx).await;
        let (allowed, notify_expired) = match outcome {
            Ok(Ok(acp::RequestPermissionOutcome::Selected(selected))) => {
                (selected.option_id.to_string() == "allow", true)
            }
            // User pressed cancel, the runner cancelled the turn, or the
            // pending map entry was dropped: deny and let the UI clear.
            Ok(_) => (false, true),
            Err(_) => {
                // Timed out: deny and retract the modal ourselves.
                if let Some((_, pending)) = permission_requests().remove(&request_id) {
                    let _ = pending.tx.send(acp::RequestPermissionOutcome::Cancelled);
                }
                (false, true)
            }
        };
        if notify_expired {
            emit_permission_event(
                &app,
                &runtime_key,
                &role_name,
                &app_session_id,
                AcpEvent::PermissionExpired {
                    request_id: request_id.clone(),
                },
            );
        }

        let response = json!({
            "id": request_ref,
            "behavior": if allowed { "allow" } else { "deny" },
            "updatedInput": input,
            "message": if allowed {
                Value::Null
            } else {
                json!("denied by user in Jockey")
            },
        });
        if writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .is_err()
        {
            break;
        }
    }

    // Bridge disconnected (claude exited/cancelled): deny everything pending.
    for request_id in registered {
        if let Some((_, pending)) = permission_requests().remove(&request_id) {
            let _ = pending.tx.send(acp::RequestPermissionOutcome::Cancelled);
            emit_permission_event(
                &app,
                &runtime_key,
                &role_name,
                &app_session_id,
                AcpEvent::PermissionExpired { request_id },
            );
        }
    }
}

fn emit_permission_event(
    app: &tauri::AppHandle,
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
    event: AcpEvent,
) {
    let seq = BRIDGE_SEQ.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit(
        "acp/stream",
        AcpStreamPayload {
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
            event: &event,
            seq,
        },
    );
}

/// Parse `"{app_session_id}:{runtime_key}:{role_name}"` (see
/// `headless_key`).
fn parse_headless_key(key: &str) -> Option<(String, String, String)> {
    let mut parts = key.splitn(3, ':');
    let app_session_id = parts.next()?.to_string();
    let runtime_key = parts.next()?.to_string();
    let role_name = parts.next()?.to_string();
    Some((app_session_id, runtime_key, role_name))
}

/// Deny every pending bridge permission for a headless session key. Used by
/// cancel/evict/shutdown paths so the UI modal retracts promptly.
pub(crate) fn cancel_permissions_for_key(key: &str) {
    if let Some((app_session_id, runtime_key, role_name)) = parse_headless_key(key) {
        cancel_permissions_for(&runtime_key, &role_name, &app_session_id);
    }
}

// ── Spawn contract for the headless runner ───────────────────────────────────

/// Convert role-bound ACP MCP servers into a Claude `--mcp-config`
/// `mcpServers` fragment: stdio servers map to command entries, http/sse map
/// to remote entries. Servers Jockey cannot express are skipped.
pub(crate) fn claude_mcp_config_servers(
    servers: &[acp::McpServer],
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for server in servers {
        match server {
            acp::McpServer::Stdio(s) => {
                let mut env = serde_json::Map::new();
                for var in &s.env {
                    env.insert(var.name.clone(), Value::String(var.value.clone()));
                }
                map.insert(
                    s.name.clone(),
                    json!({
                        "command": s.command.to_string_lossy(),
                        "args": s.args,
                        "env": Value::Object(env),
                    }),
                );
            }
            acp::McpServer::Http(s) => {
                map.insert(
                    s.name.clone(),
                    json!({
                        "type": "http",
                        "url": s.url,
                        "headers": headers_object(&s.headers),
                    }),
                );
            }
            acp::McpServer::Sse(s) => {
                map.insert(
                    s.name.clone(),
                    json!({
                        "type": "sse",
                        "url": s.url,
                        "headers": headers_object(&s.headers),
                    }),
                );
            }
            _ => {}
        }
    }
    map
}

fn headers_object(headers: &[acp::HttpHeader]) -> Value {
    let mut map = serde_json::Map::new();
    for header in headers {
        map.insert(header.name.clone(), Value::String(header.value.clone()));
    }
    Value::Object(map)
}

fn hash_key(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&value, &mut hasher);
    std::hash::Hasher::finish(&hasher)
}

/// Temp `--mcp-config` file with role servers plus the permission bridge
/// server. The file name is derived from the session key AND the serialized
/// content, so a role MCP config change produces a new path and the stream
/// session is rebuilt on the next turn.
fn ensure_session_mcp_config(key: &str, role_servers: &[acp::McpServer]) -> Option<String> {
    let port = BRIDGE_PORT.get()?;
    let token = BRIDGE_TOKEN.get()?;
    let exe = std::env::current_exe().ok()?;
    let mut servers = claude_mcp_config_servers(role_servers);
    servers.insert(
        "jockey_perm".to_string(),
        json!({
            "command": exe.to_string_lossy(),
            "args": [BRIDGE_SUBCOMMAND],
            "env": {
                "JOCKEY_PERM_PORT": port.to_string(),
                "JOCKEY_PERM_TOKEN": token,
                "JOCKEY_PERM_KEY": key,
            },
        }),
    );
    write_session_mcp_config(key, &servers)
}

/// Temp `--mcp-config` file with role servers only (auto-approve sessions:
/// role MCP works, no permission bridge server included).
pub(crate) fn claude_mcp_config_path(key: &str, role_servers: &[acp::McpServer]) -> Option<String> {
    let servers = claude_mcp_config_servers(role_servers);
    if servers.is_empty() {
        return None;
    }
    write_session_mcp_config(key, &servers)
}

fn write_session_mcp_config(key: &str, servers: &serde_json::Map<String, Value>) -> Option<String> {
    let content =
        serde_json::to_string_pretty(&json!({ "mcpServers": Value::Object(servers.clone()) }))
            .ok()?;
    // Write under the user-isolated app data dir, not the world-readable
    // temp dir: the file embeds the permission-bridge token and port, which
    // a local attacker could otherwise read to forge auto-approvals.
    let dir = super::super::adapter::app_data_dir()?;
    let path = dir.join(format!(
        "jockey-mcp-{:016x}-{:016x}.json",
        hash_key(key),
        hash_key(&content),
    ));
    std::fs::write(&path, content).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(path.to_string_lossy().into_owned())
}

/// CLI args that wire Claude's permission prompts into Jockey. The temp
/// `--mcp-config` path is derived deterministically from the session key and
/// config content so stream-process reuse (`StreamSession::matches`) stays
/// stable across turns but rebuilds when the role MCP config changes.
pub(crate) fn claude_permission_args(
    key: &str,
    role_servers: &[acp::McpServer],
) -> Result<Vec<String>, String> {
    match ensure_session_mcp_config(key, role_servers) {
        Some(path) => Ok(vec![
            "--mcp-config".to_string(),
            path,
            "--permission-prompt-tool".to_string(),
            PERMISSION_TOOL_NAME.to_string(),
        ]),
        None => Err(
            "native claude permission bridge unavailable; refusing to start a \
             permission-gated session without a bypass"
                .to_string(),
        ),
    }
}

// ── Bridge subprocess side (stdio MCP server) ────────────────────────────────

/// Entry point for `current_exe --__jockey-permission-bridge`. Runs a minimal
/// MCP stdio server exposing the `request_permission` tool, forwarding calls
/// to the app's loopback listener and blocking until an outcome arrives.
pub fn bridge_subcommand_main() {
    let port = match std::env::var("JOCKEY_PERM_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    {
        Some(port) => port,
        None => return,
    };
    let token = std::env::var("JOCKEY_PERM_TOKEN").unwrap_or_default();
    let key = std::env::var("JOCKEY_PERM_KEY").unwrap_or_default();

    let pending: Arc<Mutex<HashMap<String, std::sync::mpsc::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let conn: Arc<Mutex<Option<std::net::TcpStream>>> = Arc::new(Mutex::new(None));
    let writer: Arc<Mutex<std::io::Stdout>> = Arc::new(Mutex::new(std::io::stdout()));
    let mut call_threads: Vec<std::thread::JoinHandle<()>> = Vec::new();

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let has_id = message.get("id").is_some_and(|id| !id.is_null());

        match (method, has_id) {
            ("initialize", true) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "jockey-permission-bridge",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    },
                });
                write_rpc(&writer, &response);
            }
            ("tools/list", true) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": {
                        "tools": [{
                            "name": "request_permission",
                            "description": "Ask the Jockey user to allow or deny a tool use.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "tool_name": { "type": "string" },
                                    "input": { "type": "object" },
                                },
                                "required": ["tool_name", "input"],
                            },
                        }],
                    },
                });
                write_rpc(&writer, &response);
            }
            ("tools/call", true) => {
                let id = message["id"].clone();
                let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                let pending = pending.clone();
                let conn = conn.clone();
                let writer = writer.clone();
                let token = token.clone();
                let key = key.clone();
                call_threads.push(std::thread::spawn(move || {
                    let result = handle_tool_call(&conn, &pending, port, &token, &key, &params);
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": result.to_string() }],
                            "isError": false,
                        },
                    });
                    write_rpc(&writer, &response);
                }));
            }
            // Notifications and anything else: ignore.
            _ => {}
        }
    }

    // stdin EOF: the peer no longer sends requests. Unblock every in-flight
    // permission call (each blocked thread is waiting on its channel), then
    // drain the threads so their deny responses are written before the
    // process exits. Without this, a call awaiting a user decision would
    // hold the process open for the full 300s app-side timeout.
    if let Ok(mut map) = pending.lock() {
        for (_, sender) in map.drain() {
            let _ =
                sender.send(json!({ "behavior": "deny", "message": "permission bridge closed" }));
        }
    }
    for handle in call_threads.drain(..) {
        let _ = handle.join();
    }
}

fn write_rpc(writer: &Arc<Mutex<std::io::Stdout>>, value: &Value) {
    if let Ok(mut stdout) = writer.lock() {
        let _ = writeln!(stdout, "{value}");
        let _ = stdout.flush();
    }
}

fn handle_tool_call(
    conn: &Arc<Mutex<Option<std::net::TcpStream>>>,
    pending: &Arc<Mutex<HashMap<String, std::sync::mpsc::Sender<Value>>>>,
    port: u16,
    token: &str,
    key: &str,
    params: &Value,
) -> Value {
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tool_name = arguments
        .get("tool_name")
        .or_else(|| arguments.get("toolName"))
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string();
    let input = arguments.get("input").cloned().unwrap_or_else(|| json!({}));

    let call_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    {
        match pending.lock() {
            Ok(mut map) => {
                map.insert(call_id.clone(), tx);
            }
            Err(_) => return deny_result("permission bridge state unavailable"),
        }
    }

    // Ensure the loopback connection is alive; reconnect + spawn a reader
    // thread when a previous connection dropped.
    let connected = {
        let mut guard = match conn.lock() {
            Ok(guard) => guard,
            Err(_) => return deny_result("permission bridge connection state poisoned"),
        };
        if guard.is_none() {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(mut writer_stream) => {
                    let reader_stream = match writer_stream.try_clone() {
                        Ok(reader_stream) => reader_stream,
                        Err(_) => {
                            if let Ok(mut map) = pending.lock() {
                                map.remove(&call_id);
                            }
                            return deny_result("permission bridge socket clone failed");
                        }
                    };
                    let auth = json!({ "token": token, "key": key });
                    let _ = writeln!(writer_stream, "{auth}");
                    let _ = writer_stream.flush();
                    *guard = Some(writer_stream);
                    spawn_socket_reader(reader_stream, pending.clone(), conn.clone());
                }
                Err(error) => {
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&call_id);
                    }
                    return deny_result(&format!("cannot reach Jockey permission bridge: {error}"));
                }
            }
        }
        if let Some(stream) = guard.as_mut() {
            let request = json!({ "id": call_id, "toolName": tool_name, "input": input });
            let ok = writeln!(stream, "{request}").is_ok() && stream.flush().is_ok();
            if !ok {
                *guard = None;
            }
            Some(ok)
        } else {
            None
        }
    };
    if connected != Some(true) {
        if let Ok(mut map) = pending.lock() {
            map.remove(&call_id);
        }
        return deny_result("cannot reach Jockey permission bridge");
    }

    // App side decides within PERMISSION_TIMEOUT; allow extra slack here.
    match rx.recv_timeout(PERMISSION_TIMEOUT + BRIDGE_TIMEOUT_PAD) {
        Ok(response) => {
            let behavior = response
                .get("behavior")
                .and_then(Value::as_str)
                .unwrap_or("deny");
            if behavior == "allow" {
                json!({
                    "behavior": "allow",
                    "updatedInput": response.get("updatedInput").cloned().unwrap_or_else(|| json!({})),
                })
            } else {
                let message = response
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("denied by user in Jockey");
                json!({ "behavior": "deny", "message": message })
            }
        }
        Err(_) => {
            if let Ok(mut map) = pending.lock() {
                map.remove(&call_id);
            }
            deny_result("permission request timed out")
        }
    }
}

fn spawn_socket_reader(
    stream: std::net::TcpStream,
    pending: Arc<Mutex<HashMap<String, std::sync::mpsc::Sender<Value>>>>,
    conn: Arc<Mutex<Option<std::net::TcpStream>>>,
) {
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Ok(response) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let Some(id) = response.get("id").and_then(Value::as_str) else {
                continue;
            };
            let sender = match pending.lock() {
                Ok(mut map) => map.remove(id),
                Err(_) => break,
            };
            if let Some(sender) = sender {
                let _ = sender.send(response);
            }
        }
        // Make the next permission call establish a fresh authenticated
        // connection instead of writing to a socket that already reached EOF.
        if let Ok(mut guard) = conn.lock() {
            *guard = None;
        }
        // Socket EOF: fail every pending request so blocked threads unblock.
        if let Ok(mut map) = pending.lock() {
            for (_, sender) in map.drain() {
                let _ = sender.send(
                    json!({ "behavior": "deny", "message": "permission bridge disconnected" }),
                );
            }
        }
    });
}

fn deny_result(message: &str) -> Value {
    json!({ "behavior": "deny", "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headless_key_components() {
        let parsed = parse_headless_key("sess-1:claude-native:reviewer").unwrap();
        assert_eq!(parsed.0, "sess-1");
        assert_eq!(parsed.1, "claude-native");
        assert_eq!(parsed.2, "reviewer");
    }

    #[test]
    fn role_names_may_contain_colons() {
        let parsed = parse_headless_key("sess-1:claude-native:team:lead").unwrap();
        assert_eq!(parsed.2, "team:lead");
    }

    #[test]
    fn rejects_keys_without_runtime() {
        assert!(parse_headless_key("only-role").is_none());
    }

    #[test]
    fn converts_stdio_servers_to_claude_config() {
        let servers = vec![acp::McpServer::Stdio({
            let mut server = acp::McpServerStdio::new("chrome-devtools", "/usr/bin/npx");
            server.args = vec![
                "-y".to_string(),
                "@chrome-devtools/chrome-devtools-mcp".to_string(),
            ];
            server.env = vec![acp::EnvVariable::new("DEBUG", "1")];
            server
        })];
        let map = claude_mcp_config_servers(&servers);
        let entry = map.get("chrome-devtools").unwrap();
        assert_eq!(entry["command"], "/usr/bin/npx");
        assert_eq!(entry["args"][0], "-y");
        assert_eq!(entry["env"]["DEBUG"], "1");
    }

    #[test]
    fn converts_remote_servers_to_claude_config() {
        let servers = vec![acp::McpServer::Http(acp::McpServerHttp::new(
            "remote",
            "https://mcp.example.com",
        ))];
        let map = claude_mcp_config_servers(&servers);
        let entry = map.get("remote").unwrap();
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], "https://mcp.example.com");
    }

    #[test]
    fn skips_unknown_server_variants() {
        let map = claude_mcp_config_servers(&[]);
        assert!(map.is_empty());
    }
}
