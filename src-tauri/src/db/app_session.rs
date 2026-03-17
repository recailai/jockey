use crate::db::{ensure_default_team_id, get_state, with_db};
use crate::now_ms;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

pub(crate) fn load_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Option<String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT role_sessions_json FROM app_sessions WHERE id = ?1",
            params![app_session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .and_then(|json| serde_json::from_str::<serde_json::Map<String, Value>>(&json).ok())
    .and_then(|map| map.get(role_name).and_then(|v| v.as_str()).map(|s| s.to_string()))
}

pub(crate) fn save_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    cli_session_id: &str,
) -> Result<(), String> {
    with_db(state, |conn| {
        let existing = conn
            .query_row(
                "SELECT role_sessions_json FROM app_sessions WHERE id = ?1",
                params![app_session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(json) = existing else { return Ok(()); };
        let mut map: serde_json::Map<String, Value> =
            serde_json::from_str(&json).unwrap_or_default();
        map.insert(role_name.to_string(), Value::String(cli_session_id.to_string()));
        let updated = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE app_sessions SET role_sessions_json = ?1 WHERE id = ?2",
            params![updated, app_session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn list_app_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, team_id, active_role, selected_assistant, messages_json, created_at, last_active_at
                 FROM app_sessions ORDER BY last_active_at DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AppSession {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    team_id: row.get(2)?,
                    active_role: row.get(3)?,
                    selected_assistant: row.get(4)?,
                    messages_json: row.get(5)?,
                    created_at: row.get(6)?,
                    last_active_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub(crate) fn create_app_session(
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<AppSession, String> {
    let team_id = ensure_default_team_id(get_state(&state))?;
    let now = now_ms();
    let session = AppSession {
        id: Uuid::new_v4().to_string(),
        title: title.unwrap_or_else(|| "New Session".to_string()),
        team_id,
        active_role: "UnionAI".to_string(),
        selected_assistant: None,
        messages_json: "[]".to_string(),
        created_at: now,
        last_active_at: now,
    };
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO app_sessions (id, title, team_id, active_role, selected_assistant, messages_json, created_at, last_active_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &session.id,
                &session.title,
                &session.team_id,
                &session.active_role,
                &session.selected_assistant,
                &session.messages_json,
                session.created_at,
                session.last_active_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(session)
}

#[tauri::command]
pub(crate) fn update_app_session(
    state: State<'_, AppState>,
    id: String,
    update: AppSessionUpdate,
) -> Result<(), String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        if let Some(ref title) = update.title {
            conn.execute(
                "UPDATE app_sessions SET title = ?1, last_active_at = ?2 WHERE id = ?3",
                params![title, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(ref role) = update.active_role {
            conn.execute(
                "UPDATE app_sessions SET active_role = ?1, last_active_at = ?2 WHERE id = ?3",
                params![role, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(ref assistant) = update.selected_assistant {
            conn.execute(
                "UPDATE app_sessions SET selected_assistant = ?1, last_active_at = ?2 WHERE id = ?3",
                params![assistant, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(ref msgs) = update.messages_json {
            conn.execute(
                "UPDATE app_sessions SET messages_json = ?1, last_active_at = ?2 WHERE id = ?3",
                params![msgs, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn delete_app_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    with_db(get_state(&state), |conn| {
        conn.execute("DELETE FROM app_sessions WHERE id = ?1", params![&id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}
