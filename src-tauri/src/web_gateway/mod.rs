use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, Json,
    },
    routing::{get, post},
    Router,
};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Listener, Manager};

use crate::types::AppState;

const DEFAULT_PORT: u16 = 8420;
const COMPANION_HTML: &str = include_str!("companion.html");
const TOKEN_FILE: &str = "companion_token";
/// Upper bound on how long stop waits for in-flight requests (e.g. a long
/// `assistant_chat`) before the server future is dropped outright.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
const FORWARDED_EVENTS: &[&str] = &["acp/delta", "session/update", "acp/stream"];

#[derive(Clone, Debug)]
pub struct GatewayEvent {
    pub event_name: String,
    pub payload: String,
}

#[derive(Clone)]
struct ServerContext {
    app: AppHandle,
    // A watch rather than a fixed string so regenerating the token while the
    // server runs revokes old links immediately and ends their SSE streams.
    token: tokio::sync::watch::Receiver<String>,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
    shutdown: tokio::sync::watch::Receiver<bool>,
}

struct RunningServer {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    token_tx: tokio::sync::watch::Sender<String>,
    task: tokio::task::JoinHandle<()>,
}

pub struct GatewayManager {
    port: u16,
    token: Option<String>,
    server: Option<RunningServer>,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
    // Tauri listeners are app-global and outlive a server run; installing them
    // per start would duplicate every forwarded event after a restart.
    listeners_installed: bool,
}

impl GatewayManager {
    fn is_running(&self) -> bool {
        self.server.as_ref().is_some_and(|s| !s.task.is_finished())
    }

    fn token(&mut self) -> String {
        if self.token.is_none() {
            self.token = Some(load_or_create_token());
        }
        self.token.clone().unwrap_or_default()
    }

    fn set_token(&mut self, token: String) {
        persist_token(&token);
        if let Some(server) = &self.server {
            let _ = server.token_tx.send(token.clone());
        }
        self.token = Some(token);
    }

    fn status(&mut self) -> GatewayStatus {
        let running = self.is_running();
        let token = self.token();
        build_status(running, self.port, &token)
    }
}

static MANAGER: OnceLock<Mutex<GatewayManager>> = OnceLock::new();

fn get_manager() -> &'static Mutex<GatewayManager> {
    MANAGER.get_or_init(|| {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Mutex::new(GatewayManager {
            port: DEFAULT_PORT,
            token: None,
            server: None,
            broadcaster: tx,
            listeners_installed: false,
        })
    })
}

fn token_path() -> Option<PathBuf> {
    crate::acp::app_data_dir().map(|dir| dir.join(TOKEN_FILE))
}

fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The token is persisted so links saved on a phone keep working across app
/// restarts; it is only rotated by an explicit regenerate.
fn load_or_create_token() -> String {
    if let Some(existing) = token_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return existing;
    }
    let token = new_token();
    persist_token(&token);
    token
}

fn persist_token(token: &str) {
    let Some(path) = token_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, token) {
        eprintln!("[web_gateway] failed to persist token: {e}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub local_url: String,
    pub tailscale_url: Option<String>,
    pub lan_urls: Vec<String>,
}

/// Detect local network IPs and Tailscale IP
pub fn detect_network_ips() -> (Option<String>, Vec<String>) {
    let mut tailscale_ip = None;
    let mut lan_ips = Vec::new();

    // 1. Try running tailscale CLI
    let ts_candidates = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/opt/homebrew/bin/tailscale",
        "/usr/local/bin/tailscale",
    ];
    for bin in ts_candidates {
        if let Ok(output) = std::process::Command::new(bin).arg("ip").arg("-4").output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !s.is_empty() && s.starts_with("100.") {
                    tailscale_ip = Some(s);
                    break;
                }
            }
        }
    }

    // 2. Try ifconfig to find other IPs
    if let Ok(output) = std::process::Command::new("ifconfig").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("inet ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let ip = parts[1];
                    if ip == "127.0.0.1" {
                        continue;
                    }
                    if ip.starts_with("100.") && tailscale_ip.is_none() {
                        tailscale_ip = Some(ip.to_string());
                    } else if is_private_lan_ip(ip) && !lan_ips.iter().any(|l| l == ip) {
                        lan_ips.push(ip.to_string());
                    }
                }
            }
        }
    }

    (tailscale_ip, lan_ips)
}

/// RFC 1918 ranges only; a bare `172.` prefix would also match public
/// addresses outside 172.16.0.0/12.
fn is_private_lan_ip(ip: &str) -> bool {
    ip.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|addr| addr.is_private())
}

fn build_status(running: bool, port: u16, token: &str) -> GatewayStatus {
    let (tailscale_ip, lan_ips) = detect_network_ips();
    let local_url = format!("http://localhost:{port}/?token={token}");
    let tailscale_url = tailscale_ip.map(|ip| format!("http://{ip}:{port}/?token={token}"));
    let lan_urls = lan_ips
        .into_iter()
        .map(|ip| format!("http://{ip}:{port}/?token={token}"))
        .collect();

    GatewayStatus {
        running,
        port,
        token: token.to_string(),
        local_url,
        tailscale_url,
        lan_urls,
    }
}

// ---------------------------------------------------------------------------
// Axum HTTP Handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn verify_token(headers: &HeaderMap, query: &AuthQuery, expected: &str) -> bool {
    if let Some(ref q) = query.token {
        if constant_time_eq(q, expected) {
            return true;
        }
    }
    if let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            if constant_time_eq(token.trim(), expected) {
                return true;
            }
        }
    }
    false
}

async fn handle_index() -> Html<&'static str> {
    Html(COMPANION_HTML)
}

async fn handle_status(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(
        json!({ "ok": true, "name": "jockey-companion", "version": "0.1.0" }),
    ))
}

async fn handle_sessions(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let app_state = ctx.app.state::<AppState>();
    match crate::db::app_session::list_app_sessions(app_state, None) {
        Ok(sessions) => Ok(Json(serde_json::to_value(sessions).unwrap_or(json!([])))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_session_detail(
    State(ctx): State<ServerContext>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let app_state = ctx.app.state::<AppState>();
    match crate::db::app_session::get_app_sessions_by_ids(app_state.inner(), &[session_id]) {
        Ok(mut list) => {
            if let Some(session) = list.pop() {
                Ok(Json(serde_json::to_value(session).unwrap_or(json!({}))))
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
struct ChatInput {
    input: String,
}

async fn handle_session_chat(
    State(ctx): State<ServerContext>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(body): Json<ChatInput>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let app_state = ctx.app.state::<AppState>();
    let input = crate::types::AssistantChatInput {
        input: body.input,
        runtime_kind: None,
        app_session_id: Some(session_id),
        attachments: Vec::new(),
        context: crate::types::ChatContextOptions::default(),
    };
    match crate::chat::assistant_chat(ctx.app.clone(), app_state, input).await {
        Ok(res) => Ok(Json(serde_json::to_value(res).unwrap_or(json!({})))),
        Err(e) => Ok(Json(json!({ "ok": false, "error": e }))),
    }
}

async fn handle_permissions(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let perms = crate::acp::list_pending_permissions();
    Ok(Json(serde_json::to_value(perms).unwrap_or(json!([]))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondPermissionInput {
    request_id: String,
    option_id: String,
    cancelled: bool,
}

async fn handle_permission_respond(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(body): Json<RespondPermissionInput>,
) -> Result<Json<Value>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match crate::commands::runtime_cmd::respond_permission(
        body.request_id,
        body.option_id,
        body.cancelled,
    )
    .await
    {
        Ok(_) => {
            let _ = ctx.broadcaster.send(GatewayEvent {
                event_name: "permission_update".to_string(),
                payload: json!({ "status": "resolved" }).to_string(),
            });
            Ok(Json(json!({ "ok": true })))
        }
        Err(e) => Ok(Json(json!({ "ok": false, "error": e }))),
    }
}

async fn handle_events(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>>, StatusCode> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let rx = ctx.broadcaster.subscribe();
    let mut shutdown = ctx.shutdown.clone();
    let mut token = ctx.token.clone();
    token.mark_unchanged();
    // SSE streams never end on their own; without this, graceful shutdown would
    // wait forever on any connected client and keep the port bound.
    let stopped = async move {
        tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => {}
            _ = token.changed() => {}
        }
    };
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let sse_event = SseEvent::default().event(ev.event_name).data(ev.payload);
                    return Some((Ok::<_, std::convert::Infallible>(sse_event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .take_until(Box::pin(stopped));

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_gateway_status_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    Ok(mgr.status())
}

#[tauri::command]
pub fn regenerate_gateway_token_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    mgr.set_token(new_token());
    Ok(mgr.status())
}

fn install_event_forwarders(app: &AppHandle, mgr: &mut GatewayManager) {
    if mgr.listeners_installed {
        return;
    }
    for &name in FORWARDED_EVENTS {
        let tx = mgr.broadcaster.clone();
        app.listen_any(name, move |ev| {
            let _ = tx.send(GatewayEvent {
                event_name: name.to_string(),
                payload: ev.payload().to_string(),
            });
        });
    }
    mgr.listeners_installed = true;
}

#[tauri::command]
pub async fn start_gateway_cmd(
    app: AppHandle,
    port: Option<u16>,
    token: Option<String>,
) -> Result<GatewayStatus, String> {
    let (assigned_port, assigned_token, broadcaster) = {
        let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
        if mgr.is_running() {
            return Ok(mgr.status());
        }
        if let Some(p) = port {
            mgr.port = p;
        }
        if let Some(t) = token.filter(|t| !t.trim().is_empty()) {
            mgr.set_token(t.trim().to_string());
        }
        install_event_forwarders(&app, &mut mgr);
        (mgr.port, mgr.token(), mgr.broadcaster.clone())
    };

    // Bind before reporting success so a busy port surfaces as an error
    // instead of a "running" badge on a server that never came up.
    let addr = SocketAddr::from(([0, 0, 0, 0], assigned_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind port {assigned_port}: {e}"))?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (token_tx, token_rx) = tokio::sync::watch::channel(assigned_token);
    let server_ctx = ServerContext {
        app: app.clone(),
        token: token_rx,
        broadcaster,
        shutdown: shutdown_rx.clone(),
    };

    let router = Router::new()
        .route("/", get(handle_index))
        .route("/api/status", get(handle_status))
        .route("/api/sessions", get(handle_sessions))
        .route("/api/sessions/:id", get(handle_session_detail))
        .route("/api/sessions/:id/chat", post(handle_session_chat))
        .route("/api/permissions", get(handle_permissions))
        .route("/api/permissions/respond", post(handle_permission_respond))
        .route("/api/events", get(handle_events))
        .with_state(server_ctx);

    let task = tokio::spawn(async move {
        eprintln!("[web_gateway] running on http://{addr}");
        let mut graceful_rx = shutdown_rx.clone();
        let graceful = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = graceful_rx.wait_for(|stop| *stop).await;
        });
        let mut deadline_rx = shutdown_rx;
        let deadline = async move {
            let _ = deadline_rx.wait_for(|stop| *stop).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        };
        tokio::select! {
            res = graceful => {
                if let Err(e) = res {
                    eprintln!("[web_gateway] server error: {e}");
                }
            }
            _ = deadline => eprintln!("[web_gateway] forced shutdown after grace period"),
        }
        eprintln!("[web_gateway] stopped");
    });

    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    mgr.server = Some(RunningServer {
        shutdown_tx,
        token_tx,
        task,
    });
    Ok(mgr.status())
}

#[tauri::command]
pub async fn stop_gateway_cmd() -> Result<GatewayStatus, String> {
    let server = get_manager()
        .lock()
        .map_err(|e| e.to_string())?
        .server
        .take();
    if let Some(server) = server {
        let _ = server.shutdown_tx.send(true);
        // Await the task so the port is released before the UI can restart.
        let _ = server.task.await;
    }
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    Ok(mgr.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_identical_tokens() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abc123"));
        assert!(!constant_time_eq("", "x"));
    }

    #[test]
    fn private_lan_ip_covers_only_rfc1918() {
        assert!(is_private_lan_ip("192.168.1.20"));
        assert!(is_private_lan_ip("10.0.0.5"));
        assert!(is_private_lan_ip("172.16.0.1"));
        assert!(is_private_lan_ip("172.31.255.254"));
        assert!(!is_private_lan_ip("172.15.0.1"));
        assert!(!is_private_lan_ip("172.32.0.1"));
        assert!(!is_private_lan_ip("100.64.0.1"));
        assert!(!is_private_lan_ip("not-an-ip"));
    }

    #[test]
    fn verify_token_accepts_query_or_bearer() {
        let mut headers = HeaderMap::new();
        let none = AuthQuery { token: None };
        assert!(!verify_token(&headers, &none, "t0k"));
        let q = AuthQuery {
            token: Some("t0k".into()),
        };
        assert!(verify_token(&headers, &q, "t0k"));
        headers.insert("authorization", "Bearer t0k".parse().unwrap());
        assert!(verify_token(&headers, &none, "t0k"));
        headers.insert("authorization", "Bearer nope".parse().unwrap());
        assert!(!verify_token(&headers, &none, "t0k"));
    }
}
