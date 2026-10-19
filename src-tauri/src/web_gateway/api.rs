use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use tauri::{Emitter, Manager};

use super::{GatewayEvent, ServerContext};
use crate::types::{AppSession, AppState};

const COMPANION_HTML: &str = include_str!("companion.html");
const MARKED_JS: &str = include_str!("vendor/marked.umd.js");
/// Tauri event the desktop listens to so sessions/projects created from the
/// companion appear without a manual reload. Also forwarded to SSE clients.
pub(super) const CHANGED_EVENT: &str = "companion/changed";
const DEFAULT_ROLE: &str = "Developer";
const PREVIEW_CHARS: usize = 120;

/// Sessions with a companion-initiated turn in flight. Turns run detached from
/// the HTTP request (a phone browser drops long requests when backgrounded),
/// so this is the only guard against double-sending into one session.
fn running_sessions() -> &'static DashMap<String, ()> {
    static RUNNING: OnceLock<DashMap<String, ()>> = OnceLock::new();
    RUNNING.get_or_init(DashMap::new)
}

pub(super) fn router(ctx: ServerContext) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/:id/importable", get(list_importable))
        .route("/projects/:id/import", post(import_sessions))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/closed", get(list_closed_sessions))
        .route("/sessions/:id", get(session_detail))
        .route("/sessions/:id/chat", post(session_chat))
        .route("/sessions/:id/cancel", post(session_cancel))
        .route("/sessions/:id/reopen", post(session_reopen))
        .route("/permissions", get(list_permissions))
        .route("/permissions/respond", post(respond_permission))
        .route("/events", get(events))
        .route_layer(middleware::from_fn_with_state(ctx.clone(), require_token));

    Router::new()
        .route("/", get(index))
        .route("/assets/marked.js", get(marked_js))
        .nest("/api", api)
        .with_state(ctx)
}

// ---------------------------------------------------------------------------
// Auth & errors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct AuthQuery {
    pub(super) token: Option<String>,
}

pub(super) fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(super) fn verify_token(headers: &HeaderMap, query: &AuthQuery, expected: &str) -> bool {
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

async fn require_token(
    State(ctx): State<ServerContext>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !verify_token(&headers, &auth, &ctx.token.borrow()) {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }
    Ok(next.run(req).await)
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "ok": false, "error": self.1 }))).into_response()
    }
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn internal(msg: impl ToString) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn emit_changed(ctx: &ServerContext, kind: &str, session_id: Option<&str>) {
    let _ = ctx.app.emit(
        CHANGED_EVENT,
        json!({ "kind": kind, "sessionId": session_id }),
    );
}

// ---------------------------------------------------------------------------
// Static
// ---------------------------------------------------------------------------

async fn index() -> Html<&'static str> {
    Html(COMPANION_HTML)
}

async fn marked_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        MARKED_JS,
    )
}

async fn status() -> Json<Value> {
    Json(json!({ "ok": true, "name": "jockey-companion", "version": "0.1.0" }))
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

async fn list_projects(State(ctx): State<ServerContext>) -> ApiResult<Vec<crate::types::Project>> {
    let state = ctx.app.state::<AppState>();
    crate::db::project::list_projects_internal(&state)
        .map(Json)
        .map_err(internal)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectBody {
    name: Option<String>,
    root_path: String,
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join(rest))
            .unwrap_or_else(|| path.into()),
        None => path.into(),
    }
}

async fn create_project(
    State(ctx): State<ServerContext>,
    Json(body): Json<CreateProjectBody>,
) -> ApiResult<crate::types::Project> {
    let raw = body.root_path.trim();
    if raw.is_empty() {
        return Err(bad_request("项目路径不能为空"));
    }
    let path = expand_home(raw);
    // The desktop picks folders with a native dialog; a typed path from a phone
    // needs the existence check that dialog gives for free.
    if !path.is_dir() {
        return Err(bad_request(format!("目录不存在: {}", path.display())));
    }
    let name = body
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "project".to_string());
    let state = ctx.app.state::<AppState>();
    let project = crate::db::project::create_project_internal(
        &state,
        name,
        path.to_string_lossy().to_string(),
    )
    .map_err(bad_request)?;
    emit_changed(&ctx, "projects", None);
    Ok(Json(project))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportableQuery {
    refresh: Option<bool>,
}

async fn list_importable(
    State(ctx): State<ServerContext>,
    Path(project_id): Path<String>,
    Query(q): Query<ImportableQuery>,
) -> ApiResult<Vec<crate::importer::ImportableSession>> {
    let state = ctx.app.state::<AppState>().clone_refs();
    tokio::task::spawn_blocking(move || {
        crate::importer::scan_importable_sessions_internal(
            &state,
            &project_id,
            q.refresh.unwrap_or(false),
        )
    })
    .await
    .map_err(internal)?
    .map(Json)
    .map_err(bad_request)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportBody {
    external_session_ids: Vec<String>,
}

async fn import_sessions(
    State(ctx): State<ServerContext>,
    Path(project_id): Path<String>,
    Json(body): Json<ImportBody>,
) -> ApiResult<Vec<SessionSummary>> {
    if body.external_session_ids.is_empty() {
        return Err(bad_request("请选择要导入的会话"));
    }
    let state = ctx.app.state::<AppState>().clone_refs();
    let imported = tokio::task::spawn_blocking(move || {
        crate::importer::import_project_sessions_internal(
            &state,
            &project_id,
            Some(&body.external_session_ids),
        )
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;
    emit_changed(&ctx, "sessions", None);
    Ok(Json(imported.into_iter().map(summarize).collect()))
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary {
    id: String,
    title: String,
    active_role: String,
    runtime_kind: Option<String>,
    project_id: Option<String>,
    last_active_at: i64,
    closed_at: Option<i64>,
    message_count: usize,
    preview: Option<String>,
    running: bool,
}

fn clip_chars(text: &str, max: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &compact[..idx]),
        None => compact,
    }
}

fn summarize(session: AppSession) -> SessionSummary {
    let preview = session
        .messages
        .iter()
        .rev()
        .filter(|m| m.get("roleName").and_then(Value::as_str) != Some("event"))
        .find_map(|m| m.get("text").and_then(Value::as_str))
        .filter(|t| !t.trim().is_empty())
        .map(|t| clip_chars(t, PREVIEW_CHARS));
    SessionSummary {
        running: running_sessions().contains_key(&session.id),
        message_count: session.messages.len(),
        preview,
        id: session.id,
        title: session.title,
        active_role: session.active_role,
        runtime_kind: session.runtime_kind,
        project_id: session.project_id,
        last_active_at: session.last_active_at,
        closed_at: session.closed_at,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectFilter {
    project_id: Option<String>,
}

async fn list_sessions(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Vec<SessionSummary>> {
    let state = ctx.app.state::<AppState>();
    crate::db::app_session::list_app_sessions(state, filter.project_id)
        .map(|list| Json(list.into_iter().map(summarize).collect()))
        .map_err(internal)
}

async fn list_closed_sessions(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Vec<SessionSummary>> {
    let state = ctx.app.state::<AppState>();
    let pid = filter.project_id.filter(|p| !p.trim().is_empty());
    crate::db::app_session::list_closed_app_sessions(state)
        .map(|list| {
            Json(
                list.into_iter()
                    .filter(|s| pid.is_none() || s.project_id == pid)
                    .map(summarize)
                    .collect(),
            )
        })
        .map_err(internal)
}

fn load_session(state: &AppState, id: &str) -> Result<AppSession, ApiError> {
    crate::db::app_session::get_app_sessions_by_ids(state, &[id.to_string()])
        .map_err(internal)?
        .pop()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "session not found".into()))
}

async fn session_detail(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let state = ctx.app.state::<AppState>();
    let session = load_session(&state, &id)?;
    let mut value = serde_json::to_value(session).map_err(internal)?;
    value["running"] = json!(running_sessions().contains_key(&id));
    Ok(Json(value))
}

fn runtime_key(raw: &str) -> String {
    crate::assistant::normalize_runtime_key(raw)
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

/// Mirrors the desktop's `preferredAssistantForProject`: the project's pinned
/// engine for the persona, then the persona's own default, then any installed
/// agent — each only if it is actually available on this machine.
async fn preferred_runtime(
    state: &AppState,
    project_id: Option<&str>,
    role: &str,
) -> Option<(String, String)> {
    let catalog = tokio::task::spawn_blocking(crate::assistant::cached_assistant_catalog)
        .await
        .ok()?;
    let available: Vec<_> = catalog.into_iter().filter(|a| a.available).collect();
    let find = |want: &str| {
        let want = runtime_key(want);
        available.iter().find(|a| runtime_key(&a.key) == want)
    };
    let pin = crate::db::project_agent::load_project_agent_config(state, project_id, role)
        .ok()
        .and_then(|c| c.runtime_kind);
    let persona = crate::db::role::load_role_runtime_kind_scoped(state, role, project_id).ok();
    pin.as_deref()
        .and_then(find)
        .or_else(|| persona.as_deref().and_then(find))
        .or(available.first())
        .map(|a| (a.key.clone(), a.profile_id.clone()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionBody {
    project_id: Option<String>,
    title: Option<String>,
}

async fn create_session(
    State(ctx): State<ServerContext>,
    Json(body): Json<CreateSessionBody>,
) -> ApiResult<SessionSummary> {
    let state = ctx.app.state::<AppState>();
    let project_id = body.project_id.filter(|p| !p.trim().is_empty());
    let runtime = preferred_runtime(&state, project_id.as_deref(), DEFAULT_ROLE).await;
    let session = crate::db::app_session::create_app_session_internal(
        &state,
        body.title.as_deref().filter(|t| !t.trim().is_empty()),
        project_id.as_deref(),
        runtime.as_ref().map(|(key, _)| key.as_str()),
        runtime.as_ref().map(|(_, profile)| profile.as_str()),
        None,
    )
    .map_err(bad_request)?;
    emit_changed(&ctx, "sessions", Some(&session.id));
    Ok(Json(summarize(session)))
}

async fn session_reopen(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<SessionSummary> {
    let state = ctx.app.state::<AppState>();
    let session =
        crate::db::app_session::reopen_app_session(state, id.clone()).map_err(bad_request)?;
    emit_changed(&ctx, "sessions", Some(&id));
    Ok(Json(summarize(session)))
}

#[derive(Deserialize)]
struct ChatBody {
    input: String,
}

async fn session_chat(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<ChatBody>,
) -> ApiResult<Value> {
    let text = body.input.trim().to_string();
    if text.is_empty() {
        return Err(bad_request("消息不能为空"));
    }
    let state = ctx.app.state::<AppState>();
    let session = load_session(&state, &id)?;
    if session.closed_at.is_some() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "会话已关闭，请先恢复".into(),
        ));
    }
    // assistant_chat refuses a turn without an explicit runtime; the desktop
    // passes the session's selected agent, so do the same and fall back to the
    // preferred one for sessions that never picked any.
    let runtime = match session
        .runtime_kind
        .clone()
        .filter(|r| !r.trim().is_empty())
    {
        Some(rt) => rt,
        None => preferred_runtime(&state, session.project_id.as_deref(), &session.active_role)
            .await
            .map(|(key, _)| key)
            .ok_or_else(|| bad_request("没有可用的 Agent，请先在桌面端安装或配置"))?,
    };
    if running_sessions().insert(id.clone(), ()).is_some() {
        return Err(ApiError(StatusCode::CONFLICT, "该会话正在运行中".into()));
    }

    // Persistence is normally done by the desktop UI as it renders the turn;
    // companion turns have no desktop renderer, so the gateway writes both sides.
    if let Err(e) =
        crate::db::app_session::append_app_message_internal(&state, &id, "user", &text, None, None)
    {
        running_sessions().remove(&id);
        return Err(internal(e));
    }
    emit_changed(&ctx, "messages", Some(&id));

    tokio::spawn(run_chat_turn(
        ctx.clone(),
        id,
        text,
        runtime,
        session.active_role,
    ));
    Ok(Json(json!({ "ok": true, "accepted": true })))
}

async fn run_chat_turn(
    ctx: ServerContext,
    session_id: String,
    text: String,
    runtime: String,
    active_role: String,
) {
    let input = crate::types::AssistantChatInput {
        input: text,
        runtime_kind: Some(runtime),
        app_session_id: Some(session_id.clone()),
        attachments: Vec::new(),
        context: crate::types::ChatContextOptions::default(),
    };
    let result =
        crate::chat::assistant_chat(ctx.app.clone(), ctx.app.state::<AppState>(), input).await;

    let state = ctx.app.state::<AppState>();
    let append = |role: &str, content: &str| {
        if content.trim().is_empty() {
            return;
        }
        if let Err(e) = crate::db::app_session::append_app_message_internal(
            &state,
            &session_id,
            role,
            content,
            None,
            None,
        ) {
            eprintln!("[web_gateway] failed to persist reply: {e}");
        }
    };
    let error = match &result {
        Ok(res) if !res.role_replies.is_empty() => {
            for r in &res.role_replies {
                append(&r.role_name, &r.reply);
            }
            None
        }
        Ok(res) => {
            append(&active_role, &res.reply);
            None
        }
        Err(e) => {
            append("event", &format!("Error: {e}"));
            Some(e.clone())
        }
    };

    running_sessions().remove(&session_id);
    emit_changed(&ctx, "messages", Some(&session_id));
    let _ = ctx.broadcaster.send(GatewayEvent {
        event_name: "companion/turn".to_string(),
        payload: json!({ "appSessionId": session_id, "ok": error.is_none(), "error": error })
            .to_string(),
    });
}

async fn session_cancel(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let state = ctx.app.state::<AppState>();
    let session = load_session(&state, &id)?;
    crate::commands::runtime_cmd::cancel_acp_session(state, session.active_role, id)
        .await
        .map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

async fn list_permissions() -> ApiResult<Value> {
    serde_json::to_value(crate::acp::list_pending_permissions())
        .map(Json)
        .map_err(internal)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondPermissionBody {
    request_id: String,
    option_id: String,
    cancelled: bool,
}

async fn respond_permission(
    State(ctx): State<ServerContext>,
    Json(body): Json<RespondPermissionBody>,
) -> ApiResult<Value> {
    crate::commands::runtime_cmd::respond_permission(
        body.request_id,
        body.option_id,
        body.cancelled,
    )
    .await
    .map_err(bad_request)?;
    let _ = ctx.broadcaster.send(GatewayEvent {
        event_name: "permission_update".to_string(),
        payload: json!({ "status": "resolved" }).to_string(),
    });
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// SSE
// ---------------------------------------------------------------------------

async fn events(
    State(ctx): State<ServerContext>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
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

    Sse::new(stream).keep_alive(KeepAlive::default())
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

    #[test]
    fn clip_chars_is_char_safe_and_collapses_whitespace() {
        assert_eq!(clip_chars("a  b\n c", 10), "a b c");
        assert_eq!(clip_chars("多 Agent 协同", 3), "多 A…");
    }

    #[test]
    fn expand_home_only_rewrites_tilde_prefix() {
        assert_eq!(expand_home("/tmp/x"), std::path::PathBuf::from("/tmp/x"));
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(
                expand_home("~/code"),
                std::path::PathBuf::from(home).join("code")
            );
        }
    }
}
