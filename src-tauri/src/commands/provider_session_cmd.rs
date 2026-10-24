//! Provider handles are resumed through ACP session/load. Historical transcript discovery
//! stays in importer; fork and rewind require a negotiated ACP extension.
use crate::db::get_state;
use crate::types::AppState;
use tauri::State;

#[tauri::command]
pub(crate) async fn import_provider_session_cmd(
    state: State<'_, AppState>,
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    provider_session_id: String,
) -> Result<(), String> {
    let runtime_key = crate::runtime_profile::runtime_key_static(&runtime_kind)
        .ok_or("unsupported ACP runtime")?;
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    if provider_session_id.trim().is_empty() {
        return Err("provider session id required".to_string());
    }
    crate::db::app_session_role::save_app_session_role_cli_id(
        get_state(&state),
        app_session_id.trim(),
        runtime_key,
        &role_name,
        provider_session_id.trim(),
    )
}

#[tauri::command]
pub(crate) async fn list_provider_sessions_cmd(
    runtime_kind: String,
    limit: Option<u32>,
    cwd: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let _ = (runtime_kind, limit, cwd);
    Err("This ACP adapter has not negotiated session listing. Use Import CLI sessions.".into())
}
#[tauri::command]
pub(crate) async fn fork_provider_session_cmd(
    state: State<'_, AppState>,
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    cwd: Option<String>,
) -> Result<String, String> {
    let _ = (state, runtime_kind, role_name, app_session_id, cwd);
    Err("This ACP adapter has not negotiated session fork.".into())
}
#[tauri::command]
pub(crate) async fn rewind_provider_session_cmd(
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    num_turns: Option<u32>,
) -> Result<(), String> {
    let _ = (runtime_kind, role_name, app_session_id, num_turns);
    Err("This ACP adapter has not negotiated session rewind.".into())
}
