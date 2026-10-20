//! Desktop adapter: Tauri commands over the engine's session and project operations. Names
//! match the commands they replace, so the frontend's `invoke` calls did not change.

use tauri::State;

use super::sessions::{self, NewSession};
use crate::db::get_state;
use crate::types::{AppSession, AppSessionUpdate, AppState, Project};

#[tauri::command]
pub(crate) async fn create_app_session(
    state: State<'_, AppState>,
    title: Option<String>,
    project_id: Option<String>,
    runtime_kind: Option<String>,
    runtime_profile_id: Option<String>,
    id: Option<String>,
) -> Result<AppSession, String> {
    sessions::create_session(
        get_state(&state),
        NewSession {
            title: title.as_deref(),
            project_id: project_id.as_deref(),
            runtime_kind: runtime_kind.as_deref(),
            runtime_profile_id: runtime_profile_id.as_deref(),
            id: id.as_deref(),
        },
    )
    .await
}

#[tauri::command]
pub(crate) fn update_app_session(
    state: State<'_, AppState>,
    id: String,
    update: AppSessionUpdate,
) -> Result<(), String> {
    sessions::update_session(get_state(&state), &id, update)
}

/// Close (not delete): the row stays and can be reopened. Closing an already-closed session is
/// a no-op, which is what the desktop's close button relies on.
#[tauri::command]
pub(crate) fn delete_app_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    sessions::close_session(get_state(&state), &id).map(|_| ())
}

#[tauri::command]
pub(crate) fn reopen_app_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<AppSession, String> {
    sessions::reopen_session(get_state(&state), &id)
}

/// Sidebar rows for every open session, without transcripts. Startup loads full history only
/// for the current project; the other projects' rows come from here so the sidebar is complete.
#[tauri::command(async)]
pub(crate) fn list_session_summaries_cmd(
    state: State<'_, AppState>,
    project_id: Option<String>,
    closed: Option<bool>,
) -> Result<Vec<crate::types::SessionSummary>, String> {
    let state = get_state(&state);
    let mut rows = crate::db::app_session::list_session_summaries(
        state,
        project_id.as_deref().filter(|p| !p.trim().is_empty()),
        closed.unwrap_or(false),
        None,
    )?;
    for row in &mut rows {
        row.external_active = super::watch::is_external_active(&row.id)
            || (row.closed_at.is_some() && super::watch::provider_recently_written(state, &row.id));
    }
    Ok(rows)
}

/// Full sessions (with transcripts) by id — what a view loads when another client created
/// a session it has only seen as a `session/changed` summary.
#[tauri::command(async)]
pub(crate) fn get_app_sessions(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<AppSession>, String> {
    crate::db::app_session::get_app_sessions_by_ids(get_state(&state), &ids)
}

/// A view started showing a session: follow it (late-joined turns, provider activity outside
/// Jockey) until the matching `unwatch_session`. Returns the running turn's snapshot, if any.
///
/// The desktop shows one session at a time, so it holds a single watch that a new call
/// replaces. A window reload that never sent `unwatch_session` then cannot leak a watcher.
#[tauri::command]
pub(crate) fn watch_session(app: tauri::AppHandle, id: String) -> Option<serde_json::Value> {
    let mut current = desktop_watch().lock().unwrap_or_else(|e| e.into_inner());
    if current.as_deref() != Some(id.as_str()) {
        if let Some(previous) = current.replace(id.clone()) {
            super::watch::unwatch(&previous);
        }
        return super::watch::watch(&app, &id);
    }
    super::turns::live_turn(&id)
}

#[tauri::command]
pub(crate) fn unwatch_session(id: String) {
    let mut current = desktop_watch().lock().unwrap_or_else(|e| e.into_inner());
    if current.as_deref() == Some(id.as_str()) {
        current.take();
        super::watch::unwatch(&id);
    }
}

fn desktop_watch() -> &'static std::sync::Mutex<Option<String>> {
    static CURRENT: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
        std::sync::OnceLock::new();
    CURRENT.get_or_init(|| std::sync::Mutex::new(None))
}

/// The running turn as it would be persisted now: `{ turnId, rev, message }`.
#[tauri::command]
pub(crate) fn get_live_turn(id: String) -> Option<serde_json::Value> {
    super::turns::live_turn(&id)
}

#[tauri::command]
pub(crate) fn create_project_cmd(
    state: State<'_, AppState>,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    // Deliberately does not import transcripts. A repo can hold hundreds of CLI sessions,
    // and silently materialising all of them buries the project's real sessions. The caller
    // offers them through the import picker instead.
    sessions::create_project(get_state(&state), name, root_path)
}

#[tauri::command]
pub(crate) fn delete_project_cmd(state: State<'_, AppState>, id: String) -> Result<(), String> {
    sessions::delete_project(get_state(&state), &id)
}

#[tauri::command]
pub(crate) async fn import_project_sessions_cmd(
    state: State<'_, AppState>,
    project_id: String,
    external_session_ids: Option<Vec<String>>,
) -> Result<Vec<AppSession>, String> {
    sessions::import_sessions(get_state(&state), &project_id, external_session_ids).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliImportResult {
    sessions: Vec<AppSession>,
    created_projects: Vec<Project>,
}

/// Import sessions picked from the machine-wide list; `source_ids` are `ImportableSession.sourceId`s.
#[tauri::command]
pub(crate) async fn import_cli_sessions_cmd(
    state: State<'_, AppState>,
    source_ids: Vec<String>,
) -> Result<CliImportResult, String> {
    let (sessions, created_projects) =
        sessions::import_cli_sessions(get_state(&state), source_ids).await?;
    Ok(CliImportResult {
        sessions,
        created_projects,
    })
}
