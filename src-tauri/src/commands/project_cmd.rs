use crate::db::{
    get_state,
    project::{
        create_project_internal, delete_project_internal, get_project_internal,
        list_projects_internal,
    },
};
use crate::importer::import_project_sessions_internal;
use crate::types::{AppState, Project};
use tauri::State;

#[tauri::command]
pub(crate) fn list_projects_cmd(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    list_projects_internal(get_state(&state))
}

#[tauri::command]
pub(crate) fn get_project_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Project>, String> {
    get_project_internal(get_state(&state), &id)
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
    create_project_internal(get_state(&state), name, root_path)
}

#[tauri::command]
pub(crate) fn delete_project_cmd(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_project_internal(get_state(&state), &id)
}

#[tauri::command]
pub(crate) fn import_project_sessions_cmd(
    state: State<'_, AppState>,
    project_id: String,
    external_session_ids: Option<Vec<String>>,
) -> Result<Vec<crate::types::AppSession>, String> {
    import_project_sessions_internal(
        get_state(&state),
        &project_id,
        external_session_ids.as_deref(),
    )
}

/// What could be imported, without importing it. Backs the selection dialog.
#[tauri::command]
pub(crate) fn scan_importable_sessions_cmd(
    state: State<'_, AppState>,
    project_id: String,
    force_refresh: Option<bool>,
) -> Result<Vec<crate::importer::ImportableSession>, String> {
    crate::importer::scan_importable_sessions_internal(
        get_state(&state),
        &project_id,
        force_refresh.unwrap_or(false),
    )
}
