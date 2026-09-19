use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::{AgentLifecycle, AppState};
use rusqlite::{params, OptionalExtension};
use tauri::State;

fn valid_transition(from: Option<&str>, to: &str) -> bool {
    match (from, to) {
        (None, "idle" | "prewarming" | "running") => true,
        (Some("idle"), "prewarming" | "running" | "stopped" | "error") => true,
        (Some("prewarming"), "ready" | "error" | "stopping") => true,
        (Some("ready"), "running" | "prewarming" | "stopping" | "stopped" | "error") => true,
        (Some("running"), "ready" | "stopping" | "stopped" | "error") => true,
        (Some("stopping"), "stopped" | "ready" | "error") => true,
        (Some("stopped"), "prewarming" | "running" | "idle") => true,
        (Some("error"), "prewarming" | "running" | "stopped" | "idle") => true,
        _ => false,
    }
}

pub(crate) fn transition_internal(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
    next_state: &str,
    error: Option<&str>,
) -> Result<(), String> {
    with_db(state, |conn| {
        let current: Option<(String, i64)> = conn
            .query_row(
                "SELECT state, revision FROM agent_lifecycle
                 WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
                params![app_session_id, role_name, runtime_kind],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if !valid_transition(current.as_ref().map(|v| v.0.as_str()), next_state) {
            return Err(format!(
                "invalid agent lifecycle transition to {next_state}"
            ));
        }
        let revision = current.map(|(_, revision)| revision + 1).unwrap_or(1);
        conn.execute(
            "INSERT INTO agent_lifecycle
             (app_session_id, role_name, runtime_kind, state, revision, last_error, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
               state = excluded.state, revision = excluded.revision,
               last_error = excluded.last_error, updated_at = excluded.updated_at",
            params![
                app_session_id,
                role_name,
                runtime_kind,
                next_state,
                revision,
                error,
                now_ms()
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn list_internal(
    state: &AppState,
    app_session_id: &str,
) -> Result<Vec<AgentLifecycle>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT app_session_id, role_name, runtime_kind, state, revision, last_error, updated_at
                 FROM agent_lifecycle WHERE app_session_id = ?1 ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![app_session_id], |row| {
                Ok(AgentLifecycle {
                    app_session_id: row.get(0)?,
                    role_name: row.get(1)?,
                    runtime_kind: row.get(2)?,
                    state: row.get(3)?,
                    revision: row.get(4)?,
                    last_error: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| row.map_err(|e| e.to_string())).collect()
    })
}

#[tauri::command]
pub(crate) fn list_agent_lifecycle_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
) -> Result<Vec<AgentLifecycle>, String> {
    list_internal(get_state(&state), &app_session_id)
}
