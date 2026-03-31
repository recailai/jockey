use crate::acp;
use crate::assistant::normalize_runtime_key;
use crate::db::{get_state, with_db};
use crate::resolve_chat_cwd;
use crate::types::AppState;
use rusqlite::{params, OptionalExtension};
use tauri::State;

fn normalize_runtime_or_self(runtime: &str) -> String {
    normalize_runtime_key(runtime).unwrap_or(runtime).to_string()
}

fn resolve_runtime_for_session_role(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Result<String, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }

    if role_name == "UnionAIAssistant" {
        let session_runtime = with_db(state, |conn| {
            conn.query_row(
                "SELECT runtime_kind FROM app_sessions WHERE id = ?1",
                params![sid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
            .map(|v| v.flatten())
        })?;
        if let Some(rt) = session_runtime {
            return Ok(normalize_runtime_or_self(&rt));
        }
        return Err("assistant runtime not selected".to_string());
    }

    if let Some(row) =
        crate::db::app_session_role::load_app_session_role_state(state, sid, role_name)?
    {
        if let Some(rt) = row.runtime_kind {
            return Ok(normalize_runtime_or_self(&rt));
        }
    }
    Ok(normalize_runtime_or_self(
        &crate::db::role::load_role_runtime_kind(state, role_name)?,
    ))
}

#[tauri::command]
pub(crate) async fn cancel_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::cancel_session(&runtime, &role_name, Some(app_session_id.as_str())).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn reset_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::reset_session(&runtime, &role_name, Some(app_session_id.as_str())).await?;
    crate::db::app_session_role::clear_app_session_role_cli_id(
        get_state(&state),
        &app_session_id,
        &role_name,
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_acp_mode(
    state: State<'_, AppState>,
    role_name: String,
    mode_id: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::set_mode(
        &runtime,
        &role_name,
        &mode_id,
        Some(app_session_id.as_str()),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_acp_config_option(
    state: State<'_, AppState>,
    role_name: String,
    config_id: String,
    value: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::set_config_option(
        &runtime,
        &role_name,
        &config_id,
        &value,
        Some(app_session_id.as_str()),
    )
    .await
}

#[tauri::command]
pub(crate) fn list_available_commands_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Vec<serde_json::Value> {
    if app_session_id.trim().is_empty() {
        return vec![];
    }
    let runtime =
        match resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
    acp::list_available_commands(&app_session_id, &runtime, &role_name)
}

#[tauri::command]
pub(crate) fn list_discovered_config_options_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Vec<serde_json::Value> {
    if app_session_id.trim().is_empty() {
        return vec![];
    }
    let runtime =
        match resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
    acp::list_discovered_config_options(&app_session_id, &runtime, &role_name)
}

#[tauri::command]
pub(crate) async fn prewarm_role_config_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), &app_session_id)
        .unwrap_or_else(resolve_chat_cwd);
    Ok(acp::prewarm_role_for_config(
        &runtime,
        &role_name,
        &cwd,
        Some((get_state(&state), &app_session_id)),
    )
    .await)
}

#[tauri::command]
pub(crate) async fn respond_permission(
    request_id: String,
    option_id: String,
    cancelled: bool,
) -> Result<(), String> {
    use agent_client_protocol as acpsdk;
    let outcome = if cancelled {
        acpsdk::RequestPermissionOutcome::Cancelled
    } else {
        acpsdk::RequestPermissionOutcome::Selected(acpsdk::SelectedPermissionOutcome::new(
            acpsdk::PermissionOptionId::from(option_id),
        ))
    };
    acp::respond_to_permission(&request_id, outcome);
    Ok(())
}
