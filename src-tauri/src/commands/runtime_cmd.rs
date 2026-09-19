use crate::acp;
use crate::assistant::normalize_runtime_key;
use crate::db::app_session_role::{
    save_app_session_role_config_option_override, save_app_session_role_mode_override,
};
use crate::db::{get_state, with_db};
use crate::resolve_chat_cwd;
use crate::types::{AppState, ImageAttachment};
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

/// Session cwd first (works for any already-persisted session), then the project's own root
/// (needed for a draft session with no `app_sessions` row yet — see `project_id_override` on
/// `resolve_runtime_for_session_role`), then the generic chat cwd as a last resort.
fn resolve_discovery_cwd(state: &AppState, sid: &str, project_id_override: Option<&str>) -> String {
    if let Some(cwd) = crate::db::app_session::get_app_session_cwd(state, sid) {
        return cwd;
    }
    let project_id = project_id_override
        .map(|s| s.to_string())
        .or_else(|| crate::db::app_session::get_app_session_project_id(state, sid));
    if let Some(pid) = project_id {
        if let Ok(Some(project)) = crate::db::project::get_project_internal(state, &pid) {
            return project.root_path;
        }
    }
    resolve_chat_cwd()
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

/// Same layering as `chat::session_runtime::load_role_runtime_data`, lowest to highest:
/// persona default -> session-role binding -> project pin. The project layer must win here
/// too, or config/model discovery drifts from what a chat turn actually runs on (see the
/// `app_session_roles` module doc for why the session row alone is not authoritative).
///
/// `project_id_override` lets a caller that already knows the project (e.g. a draft session
/// with no `app_sessions` row yet, so the usual DB lookup would return `None`) supply it
/// directly — without this, persona discovery on an unsent draft session would silently skip
/// the project pin and fall back to the persona's global default.
fn resolve_runtime_for_session_role(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    project_id_override: Option<&str>,
) -> Result<String, String> {
    let sid = require_app_session_id(app_session_id)?;

    if let Some(probe_runtime) = resolve_runtime_probe(role_name) {
        return Ok(probe_runtime);
    }

    let project_id = project_id_override
        .map(|s| s.to_string())
        .or_else(|| crate::db::app_session::get_app_session_project_id(state, sid));
    let project_pin = crate::db::project_agent::load_project_agent_config(
        state,
        project_id.as_deref(),
        role_name,
    )
    .ok()
    .and_then(|cfg| cfg.runtime_kind);
    if let Some(rt) = project_pin {
        return Ok(normalize_runtime_or_self(&rt));
    }

    if let Some(row) =
        crate::db::app_session_role::load_app_session_role_state(state, sid, role_name)?
    {
        if let Some(rt) = row.runtime_kind {
            return Ok(normalize_runtime_or_self(&rt));
        }
    }
    if let Ok(rt) =
        crate::db::role::load_role_runtime_kind_scoped(state, role_name, project_id.as_deref())
    {
        return Ok(normalize_runtime_or_self(&rt));
    }

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

    Err(format!("runtime not found for role: {role_name}"))
}

#[tauri::command]
pub(crate) async fn cancel_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let state_ref = get_state(&state);
    let mut targets = crate::db::lifecycle::list_internal(state_ref, &app_session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            entry.role_name == role_name
                && matches!(entry.state.as_str(), "running" | "prewarming" | "ready")
        })
        .map(|entry| (entry.runtime_kind, entry.role_name))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        let runtime =
            resolve_runtime_for_session_role(state_ref, &app_session_id, &role_name, None)?;
        targets.push((runtime, role_name));
    }
    let mut first_error = None;
    for (runtime, role) in targets {
        let _ = crate::db::lifecycle::transition_internal(
            state_ref,
            &app_session_id,
            &role,
            &runtime,
            "stopping",
            None,
        );
        if let Err(error) =
            acp::cancel_session(&runtime, &role, Some(app_session_id.as_str())).await
        {
            first_error.get_or_insert(error);
        } else {
            let _ = crate::db::lifecycle::transition_internal(
                state_ref,
                &app_session_id,
                &role,
                &runtime,
                "stopped",
                None,
            );
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn steer_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
    prompt: String,
    attachments: Vec<ImageAttachment>,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name, None)?;
    acp::steer_session(&runtime, &role_name, Some(sid), &prompt, &attachments).await
}

#[tauri::command]
pub(crate) async fn reset_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name, None)?;
    acp::reset_session(&runtime, &role_name, Some(sid)).await?;
    crate::db::app_session_role::clear_app_session_role_cli_id(
        get_state(&state),
        sid,
        &role_name,
        &runtime,
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn reconnect_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name, None)?;
    acp::reconnect_session(&runtime, &role_name, Some(sid)).await?;
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
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name, None)?;
    if let Err(e) = acp::set_mode(&runtime, &role_name, &mode_id, Some(sid)).await {
        if !e.to_ascii_lowercase().contains("no active session") {
            return Err(e);
        }
    }
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
    let runtime = resolve_runtime_for_session_role(get_state(&state), sid, &role_name, None)?;
    if let Err(e) =
        acp::set_config_option(&runtime, &role_name, &config_id, &value, Some(sid)).await
    {
        if !e.to_ascii_lowercase().contains("no active session") {
            return Err(e);
        }
    }
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
pub(crate) async fn list_available_commands_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
    project_id: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = resolve_runtime_for_session_role(
        get_state(&state),
        sid,
        &role_name,
        project_id.as_deref(),
    )?;

    let existing = acp::list_available_commands(sid, &runtime, &role_name);
    if !existing.is_empty() {
        return Ok(existing);
    }

    let cwd = resolve_discovery_cwd(get_state(&state), sid, project_id.as_deref());
    let _ = acp::refresh_role_config_defs_with(
        &runtime,
        &role_name,
        &cwd,
        get_state(&state),
        project_id.as_deref(),
        false,
    )
    .await;

    Ok(acp::list_available_commands(sid, &runtime, &role_name))
}

#[tauri::command]
pub(crate) fn acp_metrics_snapshot_cmd() -> Vec<acp::AcpRuntimeMetrics> {
    acp::metrics_snapshot()
}

#[tauri::command]
pub(crate) fn acp_log_snapshot_cmd(limit: Option<usize>) -> Vec<acp::AcpLogEntry> {
    acp::acp_log_snapshot(limit)
}

#[tauri::command]
pub(crate) async fn active_acp_connections_cmd() -> Vec<acp::ActiveConnectionInfo> {
    acp::active_connections_snapshot().await
}

/// `force` is set only by the role editor's explicit "refresh models" action. A persona or
/// agent switch leaves it unset so the catalog is reused, instead of re-spawning the CLI on
/// every switch the way this path used to.
#[tauri::command]
pub(crate) async fn prewarm_role_config_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
    project_id: Option<String>,
    force: Option<bool>,
    runtime_kind: Option<String>,
) -> Result<serde_json::Value, String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = if let Some(rt) = runtime_kind.filter(|s| !s.trim().is_empty()) {
        normalize_runtime_or_self(&rt)
    } else {
        resolve_runtime_for_session_role(
            get_state(&state),
            sid,
            &role_name,
            project_id.as_deref(),
        )?
    };
    let cwd = resolve_discovery_cwd(get_state(&state), sid, project_id.as_deref());
    // Role/config UI discovery uses a separate refresh connection so it cannot
    // evict the live app-session connection serving the running chat.
    let (opts, modes) = acp::refresh_role_config_defs_with(
        &runtime,
        &role_name,
        &cwd,
        get_state(&state),
        project_id.as_deref(),
        force.unwrap_or(false),
    )
    .await;
    if !opts.is_empty() {
        if let Ok(serialized) = serde_json::to_string(&opts) {
            let _ = crate::db::role::update_role_config_option_defs_if_changed(
                get_state(&state),
                &role_name,
                Some(&runtime),
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
    use crate::acp::protocol as acpsdk;
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

#[tauri::command]
pub(crate) async fn reset_role_mcp_sessions_cmd(
    state: State<'_, AppState>,
    role_id: String,
) -> Result<Vec<String>, String> {
    let eligible = with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT asr.app_session_id
                 FROM app_session_roles asr
                 JOIN app_sessions s ON s.id = asr.app_session_id
                 JOIN roles r ON r.id = ?1 AND r.role_name = asr.role_name
                   AND (
                     COALESCE(r.project_id, '') = COALESCE(s.project_id, '')
                     OR (
                       COALESCE(r.project_id, '') = ''
                       AND NOT EXISTS (
                         SELECT 1 FROM roles local
                         WHERE local.role_name = asr.role_name
                           AND COALESCE(local.project_id, '') = COALESCE(s.project_id, '')
                       )
                     )
                   )",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map(params![role_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|row| row.ok())
            .collect::<Vec<_>>();
        Ok(ids)
    })
    .unwrap_or_default();
    let eligible = eligible
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let conns = acp::active_connections_snapshot().await;
    let mut reset_ids = Vec::new();
    for conn in conns {
        if !eligible.contains(&conn.app_session_id) {
            continue;
        }
        let _ = acp::reset_session(
            &conn.runtime_key,
            &conn.role_name,
            Some(&conn.app_session_id),
        )
        .await;
        reset_ids.push(conn.app_session_id);
    }
    Ok(reset_ids)
}

#[tauri::command]
pub(crate) async fn sync_role_mode_cmd(
    state: State<'_, AppState>,
    role_id: String,
    mode_id: String,
) -> Result<Vec<String>, String> {
    let (role_name, eligible) = with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT r.role_name, asr.app_session_id
                 FROM app_session_roles asr
                 JOIN app_sessions s ON s.id = asr.app_session_id
                 JOIN roles r ON r.id = ?1 AND r.role_name = asr.role_name
                   AND (
                     COALESCE(r.project_id, '') = COALESCE(s.project_id, '')
                     OR (
                       COALESCE(r.project_id, '') = ''
                       AND NOT EXISTS (
                         SELECT 1 FROM roles local
                         WHERE local.role_name = asr.role_name
                           AND COALESCE(local.project_id, '') = COALESCE(s.project_id, '')
                       )
                     )
                   )
                 WHERE asr.mode_override IS NULL OR asr.mode_override = ''",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![role_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        let role_name = conn
            .query_row(
                "SELECT role_name FROM roles WHERE id = ?1",
                params![role_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?;
        Ok((
            role_name,
            rows.into_iter().map(|(_, session_id)| session_id).collect(),
        ))
    })?;
    Ok(acp::sync_role_mode(&role_name, &mode_id, eligible).await)
}

#[tauri::command]
pub(crate) fn get_project_agent_config_cmd(
    state: State<'_, AppState>,
    project_id: Option<String>,
    role_name: String,
) -> Result<serde_json::Value, String> {
    let st = get_state(&state);
    let config =
        crate::db::project_agent::load_project_agent_config(st, project_id.as_deref(), &role_name)?;
    Ok(serde_json::json!({
        "projectId": project_id,
        "roleName": role_name,
        "runtimeKind": config.runtime_kind,
        "configOptions": config
            .options
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect::<serde_json::Map<_, _>>(),
    }))
}

/// Engines pinned for each persona in this project. The picker needs all of them at once so
/// its persona list can show the engine that will actually run, rather than each persona
/// template's own default — those differ, and showing the template one made the menu
/// contradict the Agent row directly above it.
#[tauri::command]
pub(crate) fn list_project_agent_pins_cmd(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let pins = crate::db::project_agent::list_project_agent_pins(
        get_state(&state),
        project_id.as_deref(),
    )?;
    Ok(serde_json::Value::Object(
        pins.into_iter()
            .map(|(role, runtime)| (role, serde_json::Value::String(runtime)))
            .collect(),
    ))
}

#[tauri::command]
pub(crate) fn set_project_agent_config_cmd(
    state: State<'_, AppState>,
    project_id: Option<String>,
    role_name: String,
    config_id: String,
    value: String,
    runtime_kind: Option<String>,
) -> Result<serde_json::Value, String> {
    let st = get_state(&state);
    if let Some(project_id) = project_id.as_deref().filter(|id| !id.trim().is_empty()) {
        if crate::db::project::get_project_internal(st, project_id)?.is_none() {
            return Err(format!("project not found: {project_id}"));
        }
    }
    let runtime = runtime_kind.map(|r| normalize_runtime_or_self(&r));
    let config = crate::db::project_agent::save_project_agent_config_option(
        st,
        project_id.as_deref(),
        &role_name,
        runtime.as_deref(),
        &config_id,
        &value,
    )?;
    Ok(serde_json::json!({
        "projectId": project_id,
        "roleName": role_name,
        "runtimeKind": config.runtime_kind,
        "configOptions": config
            .options
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect::<serde_json::Map<_, _>>(),
    }))
}

/// Pin the engine for this persona in this project.
#[tauri::command]
pub(crate) fn set_project_agent_runtime_cmd(
    state: State<'_, AppState>,
    project_id: Option<String>,
    role_name: String,
    runtime_kind: String,
) -> Result<serde_json::Value, String> {
    let st = get_state(&state);
    if let Some(project_id) = project_id.as_deref().filter(|id| !id.trim().is_empty()) {
        if crate::db::project::get_project_internal(st, project_id)?.is_none() {
            return Err(format!("project not found: {project_id}"));
        }
    }
    let runtime = normalize_runtime_or_self(&runtime_kind);
    let config = crate::db::project_agent::save_project_agent_runtime(
        st,
        project_id.as_deref(),
        &role_name,
        &runtime,
    )?;
    Ok(serde_json::json!({
        "projectId": project_id,
        "roleName": role_name,
        "runtimeKind": config.runtime_kind,
        "configOptions": config
            .options
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect::<serde_json::Map<_, _>>(),
    }))
}

#[tauri::command]
pub(crate) fn bind_session_agent_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
    role_name: String,
    runtime_kind: String,
) -> Result<String, String> {
    let sid = require_app_session_id(&app_session_id)?;
    let runtime = normalize_runtime_or_self(&runtime_kind);
    crate::db::app_session_role::bind_app_session_role_runtime(
        get_state(&state),
        sid,
        &role_name,
        &runtime,
    )?;
    Ok(runtime)
}

#[tauri::command]
pub(crate) fn respond_user_input_cmd(
    request_id: String,
    answers: Option<std::collections::HashMap<String, Vec<String>>>,
) -> Result<(), String> {
    crate::acp::respond_to_user_input(&request_id, answers);
    Ok(())
}
