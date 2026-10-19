use crate::db::{
    get_state,
    project::{get_project_internal, list_projects_internal},
};
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

/// What could be imported, without importing it. Backs the selection dialog. Scans provider
/// transcript files, so it must not run on the main thread.
#[tauri::command(async)]
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
