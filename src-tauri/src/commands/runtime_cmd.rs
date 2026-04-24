use crate::acp;
use crate::assistant::normalize_runtime_key;
use crate::db::app_session_role::{
    save_app_session_role_config_option_override, save_app_session_role_mode_override,
};
use crate::db::{get_state, with_db};
use crate::resolve_chat_cwd;
use crate::types::AppState;
use rusqlite::{params, OptionalExtension};
use tauri::State;

const RUNTIME_PROBE_PREFIX: &str = "runtime:";

fn require_app_session_id(sid: &str) -> Result<&str, String> {
    let trimmed = sid.trim();
    if trimmed.is_empty() {
        return Err("app session id required".to_string());
    }
    Ok(trimmed)
}

fn normalize_runtime_or_self(runtime: &str) -> String {
    normalize_runtime_key(runtime)
        .unwrap_or(runtime)
        .to_string()
}

fn resolve_runtime_probe(role_name: &str) -> Option<String> {
    role_name
        .strip_prefix(RUNTIME_PROBE_PREFIX)
        .and_then(normalize_runtime_key)
        .map(|v| v.to_string())
}

fn resolve_runtime_for_role(state: &AppState, role_name: &str) -> Result<String, String> {
    if let Some(probe_runtime) = resolve_runtime_probe(role_name) {
        return Ok(probe_runtime);
    }
    Ok(normalize_runtime_or_self(
        &crate::db::role::load_role_runtime_kind(state, role_name)?,
    ))
}

fn resolve_runtime_for_session_role(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Result<String, String> {
    let sid = require_app_session_id(app_session_id)?;

    if role_name == "Jockey" {
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

    if let Some(probe_runtime) = resolve_runtime_probe(role_name) {
        return Ok(probe_runtime);
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
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name)?;
    acp::reset_session(&runtime, &role_name, Some(sid)).await?;
    crate::db::app_session_role::clear_app_session_role_cli_id(get_state(&state), sid, &role_name)?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn reconnect_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name)?;
    acp::reconnect_session(&runtime, &role_name, Some(sid)).await?;
    crate::db::app_session_role::clear_app_session_role_cli_id(get_state(&state), sid, &role_name)?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_acp_mode(
    state: State<'_, AppState>,
    role_name: String,
    mode_id: String,
    app_session_id: String,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name)?;
    acp::set_mode(&runtime, &role_name, &mode_id, Some(sid)).await?;
    save_app_session_role_mode_override(
        get_state(&state),
        sid,
        &role_name,
        &runtime,
        Some(&mode_id),
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_acp_config_option(
    state: State<'_, AppState>,
    role_name: String,
    config_id: String,
    value: String,
    app_session_id: String,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name)?;
    acp::set_config_option(&runtime, &role_name, &config_id, &value, Some(sid)).await?;
    save_app_session_role_config_option_override(
        get_state(&state),
        sid,
        &role_name,
        &runtime,
        &config_id,
        &value,
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) fn list_available_commands_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Vec<serde_json::Value> {
    let Ok(sid) = require_app_session_id(&app_session_id) else {
        return vec![];
    };
    let runtime = match resolve_runtime_for_session_role(get_state(&state), sid, &role_name) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    acp::list_available_commands(sid, &runtime, &role_name)
}

#[tauri::command]
pub(crate) fn list_discovered_config_options_cmd(
    state: State<'_, AppState>,
    role_name: String,
) -> Vec<serde_json::Value> {
    let runtime = match resolve_runtime_for_role(get_state(&state), &role_name) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let discovered = acp::list_discovered_config_options(&runtime);
    if !discovered.is_empty() {
        return discovered;
    }
    crate::db::role::load_role(get_state(&state), &role_name)
        .ok()
        .flatten()
        .and_then(|r| {
            serde_json::from_str::<Vec<serde_json::Value>>(&r.config_option_defs_json).ok()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) fn list_discovered_modes_cmd(
    state: State<'_, AppState>,
    role_name: String,
) -> Vec<String> {
    let runtime = match resolve_runtime_for_role(get_state(&state), &role_name) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    acp::list_discovered_modes(&runtime)
}

#[tauri::command]
pub(crate) async fn prewarm_role_config_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<serde_json::Value, String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name)?;
    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), sid)
        .unwrap_or_else(resolve_chat_cwd);
    let (opts, modes) = acp::prewarm_role_for_config(
        &runtime,
        &role_name,
        &cwd,
        Some((get_state(&state), sid)),
        true,
    )
    .await;
    if !opts.is_empty() {
        if let Ok(serialized) = serde_json::to_string(&opts) {
            let _ = crate::db::role::update_role_config_option_defs_if_changed(
                get_state(&state),
                &role_name,
                &serialized,
            );
        }
    }
    Ok(serde_json::json!({ "configOptions": opts, "modes": modes }))
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
