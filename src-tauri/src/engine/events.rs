//! The engine's single event emitter. Every engine event goes out through here, to the desktop
//! webview (Tauri events) and — via the gateway's forwarder — to web clients.
//!
//! Holding the `AppHandle` here lets code that only has `&AppState` (the Jockey MCP handlers,
//! repository-level callers) announce changes too, and keeps emission a no-op in unit tests.

use serde::Serialize;
use serde_json::json;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

use crate::types::{AppState, SessionSummary};

pub(crate) const TURN_STARTED: &str = "turn/started";
pub(crate) const TURN_FINISHED: &str = "turn/finished";
pub(crate) const QUEUE_CHANGED: &str = "queue/changed";
pub(crate) const MESSAGE_ADDED: &str = "message/added";
pub(crate) const SESSION_CHANGED: &str = "session/changed";
pub(crate) const PROJECT_CHANGED: &str = "project/changed";

/// Every engine event, for forwarders that relay all of them (the web gateway).
pub(crate) const ALL: &[&str] = &[
    TURN_STARTED,
    TURN_FINISHED,
    QUEUE_CHANGED,
    MESSAGE_ADDED,
    SESSION_CHANGED,
    PROJECT_CHANGED,
];

static APP: OnceLock<AppHandle> = OnceLock::new();

pub(crate) fn install(app: AppHandle) {
    let _ = APP.set(app);
}

pub(crate) fn emit<P: Serialize + Clone>(event: &str, payload: P) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessionChange {
    Created,
    Updated,
    Closed,
    Reopened,
    Imported,
}

/// Announce a session change with its current summary row, so views patch their lists in
/// place instead of refetching (see docs/engine-service-layer.md §2).
pub(crate) fn session_changed(state: &AppState, session_id: &str, reason: SessionChange) {
    let summary: Option<SessionSummary> = crate::db::app_session::list_session_summaries(
        state,
        None,
        false,
        Some(&[session_id.to_string()]),
    )
    .ok()
    .and_then(|mut rows| rows.pop())
    .map(|mut row| {
        row.running = super::turns::is_busy(&row.id);
        row
    });
    emit(
        SESSION_CHANGED,
        json!({
            "appSessionId": session_id,
            "projectId": summary.as_ref().and_then(|s| s.project_id.clone()),
            "reason": reason,
            "session": summary,
        }),
    );
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProjectChange {
    Created,
    Deleted,
}

pub(crate) fn project_changed(project_id: &str, reason: ProjectChange) {
    emit(
        PROJECT_CHANGED,
        json!({ "projectId": project_id, "reason": reason }),
    );
}
