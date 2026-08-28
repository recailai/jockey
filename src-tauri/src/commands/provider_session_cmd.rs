//! Provider session administration commands (list / import / fork / rewind).
//!
//! These expose the native provider handles that Jockey persists per
//! app-session role (the same `app_session_roles.acp_session_id` used for cold-start
//! resume), so the UI can list a provider's past threads, bind one to a
//! session role, fork it, or rewind the live thread.
//!
//! Verified protocol surface:
//! - native Codex app-server v2: `thread/list`, `thread/fork`,
//!   `thread/rollback`, `thread/resume` (list + fork + rewind + import).
//! - native Pi RPC 0.84+: resume via `--session <id>` (import only; the RPC
//!   layer has no list/fork/rollback methods).

use crate::acp::resolve_adapter_launch;
use crate::acp::session::ProviderThreadSummary;
use crate::acp::session::{fork_codex_thread, list_codex_threads};
use crate::db::{get_state, with_db};
use crate::resolve_chat_cwd;
use crate::types::AppState;
use rusqlite::OptionalExtension;
use tauri::State;

fn require_native_codex(runtime_kind: &str) -> Result<&'static str, String> {
    if crate::acp::session::supports_provider_sessions(runtime_kind) {
        crate::runtime_profile::runtime_key_static(runtime_kind)
            .ok_or_else(|| "unsupported runtime".to_string())
    } else {
        Err(format!(
            "provider session management is not supported for runtime '{runtime_kind}'"
        ))
    }
}

fn require_import_capable_runtime(runtime_kind: &str) -> Result<&'static str, String> {
    // Codex binds via thread/resume; Pi binds via `--session <id>`.
    if crate::acp::session::supports_session_import(runtime_kind) {
        crate::runtime_profile::runtime_key_static(runtime_kind)
            .ok_or_else(|| "unsupported runtime".to_string())
    } else {
        Err(format!(
            "provider session import is not supported for runtime '{runtime_kind}'"
        ))
    }
}

/// List recent provider threads (native Codex).
#[tauri::command]
pub(crate) async fn list_provider_sessions_cmd(
    runtime_kind: String,
    limit: Option<u32>,
    cwd: Option<String>,
) -> Result<Vec<ProviderThreadSummary>, String> {
    require_native_codex(&runtime_kind)?;
    let adapter = resolve_adapter_launch(&runtime_kind)?;
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let cwd = cwd.unwrap_or_else(resolve_chat_cwd);
    list_codex_threads(&adapter.binary, &adapter.args, &adapter.env, &cwd, limit).await
}

/// Bind a provider session handle to an app-session role (import). The next
/// turn on that role resumes the bound provider thread.
#[tauri::command]
pub(crate) async fn import_provider_session_cmd(
    state: State<'_, AppState>,
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    provider_session_id: String,
) -> Result<(), String> {
    let runtime_key = require_import_capable_runtime(&runtime_kind)?;
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

/// Fork the provider thread currently bound to an app-session role and rebind
/// the role to the forked thread. Returns the new provider session id.
#[tauri::command]
pub(crate) async fn fork_provider_session_cmd(
    state: State<'_, AppState>,
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    cwd: Option<String>,
) -> Result<String, String> {
    require_native_codex(&runtime_kind)?;
    let adapter = resolve_adapter_launch(&runtime_kind)?;
    let runtime_key_static = crate::runtime_profile::runtime_key_static(&runtime_kind)
        .ok_or_else(|| "unsupported runtime".to_string())?;
    let sid = app_session_id.trim().to_string();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }
    let bound = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT acp_session_id FROM app_session_roles
             WHERE app_session_id = ?1 AND role_name = ?2
             AND acp_session_id IS NOT NULL AND trim(acp_session_id) != ''",
            rusqlite::params![&sid, &role_name],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|v| v.flatten())
    })?
    .ok_or_else(|| "no provider session bound to this role".to_string())?;
    let cwd = cwd.unwrap_or_else(resolve_chat_cwd);
    let new_id =
        fork_codex_thread(&adapter.binary, &adapter.args, &adapter.env, &cwd, &bound).await?;
    crate::db::app_session_role::save_app_session_role_cli_id(
        get_state(&state),
        &sid,
        runtime_key_static,
        &role_name,
        &new_id,
    )?;
    Ok(new_id)
}

/// Roll back the live provider thread for an app-session role by `num_turns`.
#[tauri::command]
pub(crate) async fn rewind_provider_session_cmd(
    runtime_kind: String,
    role_name: String,
    app_session_id: String,
    num_turns: Option<u32>,
) -> Result<(), String> {
    require_native_codex(&runtime_kind)?;
    let runtime_key_static = crate::runtime_profile::runtime_key_static(&runtime_kind)
        .ok_or_else(|| "unsupported runtime".to_string())?;
    crate::acp::session::rollback_live_codex_thread(
        runtime_key_static,
        &role_name,
        app_session_id.trim(),
        num_turns.unwrap_or(1).max(1),
    )
    .await
}
