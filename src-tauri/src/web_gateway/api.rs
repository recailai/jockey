use axum::{
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

use super::{GatewayEvent, ServerContext};
use crate::engine::sessions as engine_sessions;
use crate::types::{AppSession, AppState, SessionSummary};

const MAX_DIR_ENTRIES: usize = 500;

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
        .route("/sessions/:id/queue", get(session_queue))
        .route(
            "/sessions/:id/queue/:item/remove",
            post(session_queue_remove),
        )
        .route("/sessions/:id/send-now", post(session_send_now))
        .route("/sessions/:id/reopen", post(session_reopen))
        .route("/sessions/:id/close", post(session_close))
        .route("/sessions/:id/role", post(session_select_role))
        .route("/sessions/:id/live", get(session_live))
        .route("/sessions/:id/reconnect", post(session_reconnect))
        .route("/sessions/:id/reset", post(session_reset))
        .route("/roles", get(list_roles))
        .route("/assistants", get(list_assistants))
        .route("/fs/dirs", get(list_dirs))
        .route("/permissions", get(list_permissions))
        .route("/pending", get(list_pending))
        .route("/permissions/respond", post(respond_permission))
        .route("/user-input/respond", post(respond_user_input))
        .route("/events", get(events))
        .route_layer(middleware::from_fn_with_state(ctx.clone(), require_token));

    Router::new()
        .nest("/api", api)
        .fallback(get(super::companion_assets::serve))
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

// ---------------------------------------------------------------------------
// Static
// ---------------------------------------------------------------------------

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
    // Opening a folder that is already a project selects it; it must not rename it to the
    // folder name the way an explicit "create with this name" would.
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if let Some(existing) =
        crate::db::project::find_project_by_path_internal(&state, &canonical.to_string_lossy())
            .map_err(internal)?
    {
        return Ok(Json(existing));
    }
    let project = engine_sessions::create_project(&state, name, path.to_string_lossy().to_string())
        .map_err(bad_request)?;
    Ok(Json(project))
}

#[derive(Deserialize)]
struct DirQuery {
    path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirEntry {
    name: String,
    path: String,
    is_git_repo: bool,
    project_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirListing {
    path: String,
    parent: Option<String>,
    home: Option<String>,
    entries: Vec<DirEntry>,
}

/// Where a folder picker should open: the directory most existing projects live in
/// (typically `~/Documents/GitHub`), else the home directory.
fn default_browse_root(projects: &[crate::types::Project]) -> Option<std::path::PathBuf> {
    let mut counts: std::collections::HashMap<std::path::PathBuf, usize> = Default::default();
    for p in projects {
        if let Some(parent) = std::path::Path::new(&p.root_path).parent() {
            *counts.entry(parent.to_path_buf()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(dir, _)| dir.is_dir())
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(dir, _)| dir)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
}

fn read_dir_listing(
    requested: Option<String>,
    projects: Vec<crate::types::Project>,
) -> Result<DirListing, String> {
    let dir = match requested
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(raw) => expand_home(raw),
        None => default_browse_root(&projects).ok_or("no starting folder")?,
    };
    let dir = std::fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let by_root: std::collections::HashMap<&str, &str> = projects
        .iter()
        .map(|p| (p.root_path.as_str(), p.id.as_str()))
        .collect();
    let mut entries: Vec<DirEntry> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let path_str = path.to_string_lossy().to_string();
            Some(DirEntry {
                is_git_repo: path.join(".git").exists(),
                project_id: by_root.get(path_str.as_str()).map(|id| id.to_string()),
                name,
                path: path_str,
            })
        })
        .collect();
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries.truncate(MAX_DIR_ENTRIES);
    Ok(DirListing {
        parent: dir.parent().map(|p| p.to_string_lossy().to_string()),
        home: std::env::var("HOME").ok(),
        path: dir.to_string_lossy().to_string(),
        entries,
    })
}

/// Folder picker for "open project" on a phone, where typing an absolute path is the hard
/// part. Directories only, dot-folders hidden; see docs/architecture/engine-service-layer.md §6.
async fn list_dirs(
    State(ctx): State<ServerContext>,
    Query(q): Query<DirQuery>,
) -> ApiResult<DirListing> {
    let projects = crate::db::project::list_projects_internal(&ctx.app.state::<AppState>())
        .map_err(internal)?;
    tokio::task::spawn_blocking(move || read_dir_listing(q.path, projects))
        .await
        .map_err(internal)?
        .map(Json)
        .map_err(bad_request)
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
    let state = ctx.app.state::<AppState>();
    let imported =
        engine_sessions::import_sessions(&state, &project_id, Some(body.external_session_ids))
            .await
            .map_err(bad_request)?;
    let ids: Vec<String> = imported.into_iter().map(|s| s.id).collect();
    crate::db::app_session::list_session_summaries(&state, None, false, Some(&ids))
        .map(Json)
        .map_err(internal)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectFilter {
    project_id: Option<String>,
}

async fn list_sessions(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Vec<SessionSummary>> {
    engine_sessions::summaries(
        &ctx.app.state::<AppState>(),
        filter.project_id.as_deref(),
        false,
    )
    .map(Json)
    .map_err(internal)
}

async fn list_closed_sessions(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Vec<SessionSummary>> {
    engine_sessions::summaries(
        &ctx.app.state::<AppState>(),
        filter.project_id.as_deref(),
        true,
    )
    .map(Json)
    .map_err(internal)
}

fn load_session(state: &AppState, id: &str) -> Result<AppSession, ApiError> {
    crate::db::app_session::get_app_sessions_by_ids(state, &[id.to_string()])
        .map_err(internal)?
        .pop()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "session not found".into()))
}

#[derive(Deserialize)]
struct DetailQuery {
    tail: Option<usize>,
}

/// One session with its transcript; `?tail=N` keeps only the last N messages so opening a
/// long session on a phone does not ship its whole history.
async fn session_detail(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Query(q): Query<DetailQuery>,
) -> ApiResult<Value> {
    let state = ctx.app.state::<AppState>().clone_refs();
    let sid = id.clone();
    // Loading and decoding a transcript is blocking work; keep it off the async workers.
    let mut session = tokio::task::spawn_blocking(move || load_session(&state, &sid))
        .await
        .map_err(internal)??;
    let total = session.messages.len();
    if let Some(tail) = q.tail.filter(|t| *t < total) {
        session.messages.drain(..total - tail);
    }
    let mut value = serde_json::to_value(session).map_err(internal)?;
    value["running"] = json!(crate::engine::turns::is_busy(&id));
    value["totalMessages"] = json!(total);
    Ok(Json(value))
}

fn summary_of(state: &AppState, id: &str) -> Result<SessionSummary, ApiError> {
    crate::db::app_session::list_session_summaries(state, None, false, Some(&[id.to_string()]))
        .map_err(internal)?
        .pop()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "session not found".into()))
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
    let session = engine_sessions::create_session(
        &state,
        engine_sessions::NewSession {
            title: body.title.as_deref(),
            project_id: body.project_id.as_deref(),
            runtime_kind: None,
            runtime_profile_id: None,
            id: None,
        },
    )
    .await
    .map_err(bad_request)?;
    summary_of(&state, &session.id).map(Json)
}

async fn session_reopen(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<SessionSummary> {
    let state = ctx.app.state::<AppState>();
    engine_sessions::reopen_session(&state, &id).map_err(bad_request)?;
    summary_of(&state, &id).map(Json)
}

async fn session_close(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    if crate::engine::turns::is_busy(&id) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "会话正在运行，请先停止".into(),
        ));
    }
    engine_sessions::close_session(&ctx.app.state::<AppState>(), &id).map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleOption {
    role_name: String,
    runtime_kind: String,
    project_scoped: bool,
}

/// Personas a session in `?projectId=` can switch to. Only what a picker needs; editing
/// personas stays on the desktop.
async fn list_roles(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Vec<RoleOption>> {
    let roles = engine_sessions::available_roles(
        &ctx.app.state::<AppState>(),
        filter.project_id.as_deref(),
    )
    .map_err(internal)?;
    Ok(Json(
        roles
            .into_iter()
            .map(|r| RoleOption {
                project_scoped: r.project_id.as_deref().is_some_and(|p| !p.is_empty()),
                runtime_kind: r.runtime_kind,
                role_name: r.role_name,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct SelectRoleBody {
    role: String,
}

async fn session_select_role(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<SelectRoleBody>,
) -> ApiResult<SessionSummary> {
    let state = ctx.app.state::<AppState>();
    engine_sessions::select_role(&state, &id, body.role.trim()).map_err(bad_request)?;
    summary_of(&state, &id).map(Json)
}

/// The running turn as it would be persisted now, for a client that opened mid-turn.
async fn session_live(Path(id): Path<String>) -> Json<Value> {
    Json(crate::engine::turns::live_turn(&id).unwrap_or(Value::Null))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatBody {
    input: String,
    #[serde(default)]
    attachments: Vec<crate::types::ImageAttachment>,
    user_message_id: Option<String>,
    assistant_message_id: Option<String>,
    runtime_kind: Option<String>,
}

/// Same front door as the desktop: idle sessions start a turn, busy ones queue the input.
async fn session_chat(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<ChatBody>,
) -> ApiResult<crate::engine::service::SubmitResult> {
    crate::engine::service::submit(
        &ctx.app,
        crate::engine::service::SubmitInput {
            session_id: id,
            text: body.input,
            attachments: body.attachments,
            user_message_id: body.user_message_id,
            assistant_message_id: body.assistant_message_id,
            runtime_kind: body.runtime_kind,
        },
    )
    .await
    .map(Json)
    .map_err(bad_request)
}

#[derive(Deserialize)]
struct RuntimeRoleBody {
    role: String,
}

async fn session_reconnect(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<RuntimeRoleBody>,
) -> ApiResult<Value> {
    crate::commands::runtime_cmd::reconnect_acp_session(ctx.app.state(), body.role, id)
        .await
        .map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

async fn session_reset(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<RuntimeRoleBody>,
) -> ApiResult<Value> {
    crate::commands::runtime_cmd::reset_acp_session(ctx.app.state(), body.role, id)
        .await
        .map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

/// Which agent CLIs this machine can run, with their capabilities; the composer and the
/// queue's delivery options depend on it.
async fn list_assistants() -> ApiResult<Vec<crate::types::AssistantRuntime>> {
    tokio::task::spawn_blocking(crate::assistant::cached_assistant_catalog)
        .await
        .map(Json)
        .map_err(internal)
}

async fn session_cancel(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let cancelled = crate::engine::service::cancel(&ctx.app, &id)
        .await
        .map_err(bad_request)?;
    Ok(Json(json!({ "ok": true, "cancelled": cancelled })))
}

async fn session_queue(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Vec<crate::types::InboxMessage>> {
    let state = ctx.app.state::<AppState>();
    crate::db::inbox::list_inbox_internal(&state, &id, false)
        .map(Json)
        .map_err(internal)
}

async fn session_queue_remove(
    State(ctx): State<ServerContext>,
    Path((id, item)): Path<(String, String)>,
) -> ApiResult<Value> {
    let state = ctx.app.state::<AppState>();
    crate::engine::queue::remove(&ctx.app, &state, &id, &item).map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

async fn session_send_now(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    crate::engine::service::send_queued_now(&ctx.app, &id)
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
struct PendingQuery {
    session_id: Option<String>,
}

/// Prompts the agent is blocked on right now, for clients that missed the live event.
async fn list_pending(Query(q): Query<PendingQuery>) -> ApiResult<Value> {
    serde_json::to_value(crate::acp::list_pending_prompts(q.session_id.as_deref()))
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
        event_name: "permission_update",
        payload: std::sync::Arc::from(json!({ "status": "resolved" }).to_string()),
        focus: None,
    });
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondUserInputBody {
    request_id: String,
    answers: Option<std::collections::HashMap<String, Vec<String>>>,
}

async fn respond_user_input(Json(body): Json<RespondUserInputBody>) -> ApiResult<Value> {
    crate::commands::runtime_cmd::respond_user_input_cmd(body.request_id, body.answers)
        .map_err(bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// SSE
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EventsQuery {
    /// The session this client is showing; only its stream frames are delivered.
    focus: Option<String>,
}

/// SSE: lifecycle events to everyone, stream frames only to the focused client
/// (docs/architecture/engine-service-layer.md §4). A client that lags past the buffer gets `resync`.
struct WatchGuard(String);

impl WatchGuard {
    fn new(app: &tauri::AppHandle, session_id: &str) -> Self {
        crate::engine::watch::watch(app, session_id);
        Self(session_id.to_string())
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        crate::engine::watch::unwatch(&self.0);
    }
}

async fn events(
    State(ctx): State<ServerContext>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let rx = ctx.broadcaster.subscribe();
    let focus: Option<std::sync::Arc<str>> = q.focus.filter(|f| !f.is_empty()).map(Into::into);
    // The focused session is followed for as long as this connection lives.
    let watching = focus.as_deref().map(|sid| WatchGuard::new(&ctx.app, sid));
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
    let stream = futures::stream::unfold(
        (rx, focus, watching),
        |(mut rx, focus, watching)| async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if ev.focus.is_some() && ev.focus != focus {
                            continue;
                        }
                        let sse_event = SseEvent::default().event(ev.event_name).data(&*ev.payload);
                        return Some((
                            Ok::<_, std::convert::Infallible>(sse_event),
                            (rx, focus, watching),
                        ));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let resync = SseEvent::default().event("resync").data("{}");
                        return Some((Ok(resync), (rx, focus, watching)));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    )
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
    fn dir_listing_hides_dotfiles_and_marks_repos_and_projects() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("repo/.git")).unwrap();
        std::fs::create_dir_all(root.path().join("plain")).unwrap();
        std::fs::create_dir_all(root.path().join(".hidden")).unwrap();
        std::fs::write(root.path().join("file.txt"), "x").unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let project = crate::types::Project {
            id: "p1".into(),
            name: "plain".into(),
            root_path: root_path.join("plain").to_string_lossy().to_string(),
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        };
        let listing = read_dir_listing(
            Some(root_path.to_string_lossy().to_string()),
            vec![project.clone()],
        )
        .unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["plain", "repo"]);
        assert!(listing.entries[1].is_git_repo);
        assert_eq!(listing.entries[0].project_id.as_deref(), Some("p1"));
        // With no path, open where the existing projects live.
        let default = read_dir_listing(None, vec![project]).unwrap();
        assert_eq!(default.path, root_path.to_string_lossy());
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
