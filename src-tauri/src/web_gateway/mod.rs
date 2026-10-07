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
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Listener, Manager};
use tower_http::cors::CorsLayer;

use crate::types::AppState;

const DEFAULT_PORT: u16 = 8420;
const COMPANION_HTML: &str = include_str!("companion.html");

#[derive(Clone, Debug)]
pub struct GatewayEvent {
    pub event_name: String,
    pub payload: String,
}

#[derive(Clone)]
struct ServerContext {
    app: AppHandle,
    token: String,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
}

pub struct GatewayManager {
    running: bool,
    port: u16,
    token: String,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
}

static MANAGER: OnceLock<Mutex<GatewayManager>> = OnceLock::new();

fn get_manager() -> &'static Mutex<GatewayManager> {
    MANAGER.get_or_init(|| {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        let default_token = uuid::Uuid::new_v4().simple().to_string();
        Mutex::new(GatewayManager {
            running: false,
            port: DEFAULT_PORT,
            token: default_token,
            shutdown_tx: None,
            broadcaster: tx,
        })
    })
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
                    } else if ip.starts_with("192.168.")
                        || ip.starts_with("10.")
                        || ip.starts_with("172.")
                    {
                        if !lan_ips.contains(&ip.to_string()) {
                            lan_ips.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }

    (tailscale_ip, lan_ips)
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

fn verify_token(headers: &HeaderMap, query: &AuthQuery, expected: &str) -> bool {
    if let Some(ref q) = query.token {
        if q == expected {
            return true;
        }
    }
    if let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            if token.trim() == expected {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
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
    if !verify_token(&headers, &auth, &ctx.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let rx = ctx.broadcaster.subscribe();
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
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_gateway_status_cmd() -> Result<GatewayStatus, String> {
    let mgr = get_manager().lock().map_err(|e| e.to_string())?;
    Ok(build_status(mgr.running, mgr.port, &mgr.token))
}

#[tauri::command]
pub fn regenerate_gateway_token_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    mgr.token = uuid::Uuid::new_v4().simple().to_string();
    Ok(build_status(mgr.running, mgr.port, &mgr.token))
}

#[tauri::command]
pub async fn start_gateway_cmd(
    app: AppHandle,
    port: Option<u16>,
    token: Option<String>,
) -> Result<GatewayStatus, String> {
    let (tx_shutdown, rx_shutdown) = tokio::sync::oneshot::channel::<()>();

    let (assigned_port, assigned_token, broadcaster) = {
        let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
        if mgr.running {
            return Ok(build_status(true, mgr.port, &mgr.token));
        }

        if let Some(p) = port {
            mgr.port = p;
        }
        if let Some(t) = token {
            if !t.trim().is_empty() {
                mgr.token = t;
            }
        }

        mgr.running = true;
        mgr.shutdown_tx = Some(tx_shutdown);
        (mgr.port, mgr.token.clone(), mgr.broadcaster.clone())
    };

    let server_ctx = ServerContext {
        app: app.clone(),
        token: assigned_token.clone(),
        broadcaster: broadcaster.clone(),
    };

    // Forward Tauri events to the SSE broadcaster
    {
        let tx = broadcaster.clone();
        app.listen_any("acp/delta", move |ev| {
            let _ = tx.send(GatewayEvent {
                event_name: "acp/delta".to_string(),
                payload: ev.payload().to_string(),
            });
        });

        let tx = broadcaster.clone();
        app.listen_any("session/update", move |ev| {
            let _ = tx.send(GatewayEvent {
                event_name: "session/update".to_string(),
                payload: ev.payload().to_string(),
            });
        });

        let tx = broadcaster.clone();
        app.listen_any("acp/stream", move |ev| {
            let _ = tx.send(GatewayEvent {
                event_name: "acp/stream".to_string(),
                payload: ev.payload().to_string(),
            });
        });
    }

    let router = Router::new()
        .route("/", get(handle_index))
        .route("/api/status", get(handle_status))
        .route("/api/sessions", get(handle_sessions))
        .route("/api/sessions/:id", get(handle_session_detail))
        .route("/api/sessions/:id/chat", post(handle_session_chat))
        .route("/api/permissions", get(handle_permissions))
        .route("/api/permissions/respond", post(handle_permission_respond))
        .route("/api/events", get(handle_events))
        .layer(CorsLayer::permissive())
        .with_state(server_ctx);

    let addr = SocketAddr::from(([0, 0, 0, 0], assigned_port));
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[web_gateway] bind error on {addr}: {e}");
                if let Ok(mut mgr) = get_manager().lock() {
                    mgr.running = false;
                }
                return;
            }
        };

        eprintln!("[web_gateway] running on http://{addr}");
        let server = axum::serve(listener, router);
        let graceful = server.with_graceful_shutdown(async move {
            let _ = rx_shutdown.await;
        });

        if let Err(e) = graceful.await {
            eprintln!("[web_gateway] server error: {e}");
        }
        eprintln!("[web_gateway] stopped");
    });

    Ok(build_status(true, assigned_port, &assigned_token))
}

#[tauri::command]
pub fn stop_gateway_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    if let Some(tx) = mgr.shutdown_tx.take() {
        let _ = tx.send(());
    }
    mgr.running = false;
    Ok(build_status(false, mgr.port, &mgr.token))
}
