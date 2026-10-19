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

/// Full sessions (with transcripts) by id — what a view loads when another client created
/// a session it has only seen as a `session/changed` summary.
#[tauri::command(async)]
pub(crate) fn get_app_sessions(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<AppSession>, String> {
    crate::db::app_session::get_app_sessions_by_ids(get_state(&state), &ids)
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
