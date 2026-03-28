use crate::db::{get_state, with_db};
use crate::{default_chat_cwd, now_ms};
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

pub(crate) fn get_app_session_cwd(state: &AppState, session_id: &str) -> Option<String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT cwd FROM app_sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .flatten()
}

pub(crate) fn set_app_session_cwd(
    state: &AppState,
    session_id: &str,
    cwd: &str,
) -> Result<(), String> {
    let now = crate::now_ms();
    with_db(state, |conn| {
        conn.execute(
            "UPDATE app_sessions SET cwd = ?1, last_active_at = ?2 WHERE id = ?3",
            params![cwd, now, session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn query_sessions(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<AppSession>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let sessions: Vec<(String, AppSession)> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            Ok((
                id.clone(),
                AppSession {
                    id,
                    title: row.get(1)?,
                    active_role: row.get(2)?,
                    runtime_kind: row.get(3)?,
                    cwd: row.get(4)?,
                    messages: Vec::new(),
                    created_at: row.get(5)?,
                    last_active_at: row.get(6)?,
                    closed_at: row.get(7)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let ids_list = sessions
        .iter()
        .map(|(id, _)| format!("'{}'", id.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");

    let mut msgs_by_session: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();

    if !ids_list.is_empty() {
        let msg_sql = format!(
            "SELECT session_id, role_name, content, created_at
             FROM app_session_messages WHERE session_id IN ({}) ORDER BY session_id, id ASC",
            ids_list
        );
        let mut msg_stmt = conn.prepare(&msg_sql).map_err(|e| e.to_string())?;
        for row in msg_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
        {
            let (session_id, role_name, content, at) = row;
            let v = serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "roleName": role_name,
                "text": content,
                "at": at
            });
            msgs_by_session.entry(session_id).or_default().push(v);
        }
    }

    let mut out = Vec::with_capacity(sessions.len());
    for (session_id, mut session) in sessions {
        session.messages = msgs_by_session.remove(&session_id).unwrap_or_default();
        out.push(session);
    }
    Ok(out)
}

#[tauri::command]
pub(crate) fn list_app_sessions(state: State<'_, AppState>) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        query_sessions(
            conn,
            "SELECT id, title, active_role, runtime_kind, cwd, created_at, last_active_at, closed_at
             FROM app_sessions WHERE closed_at IS NULL ORDER BY last_active_at DESC LIMIT 50",
        )
    })
}

#[tauri::command]
pub(crate) fn list_closed_app_sessions(state: State<'_, AppState>) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        query_sessions(
            conn,
            "SELECT id, title, active_role, runtime_kind, cwd, created_at, last_active_at, closed_at
             FROM app_sessions WHERE closed_at IS NOT NULL ORDER BY closed_at DESC LIMIT 200",
        )
    })
}

#[tauri::command]
pub(crate) fn append_app_message(
    state: State<'_, AppState>,
    session_id: String,
    role_name: String,
    content: String,
) -> Result<(), String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO app_session_messages (session_id, role_name, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![&session_id, &role_name, &content, now],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE app_sessions SET last_active_at = ?1 WHERE id = ?2",
            params![now, &session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn create_app_session(
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<AppSession, String> {
    let now = now_ms();
    let session = AppSession {
        id: Uuid::new_v4().to_string(),
        title: title.unwrap_or_else(|| "New Session".to_string()),
        active_role: "UnionAI".to_string(),
        runtime_kind: None,
        cwd: Some(default_chat_cwd()),
        messages: Vec::new(),
        created_at: now,
        last_active_at: now,
        closed_at: None,
    };
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO app_sessions (id, title, active_role, runtime_kind, cwd, created_at, last_active_at, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                &session.id,
                &session.title,
                &session.active_role,
                &session.runtime_kind,
                &session.cwd,
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
        if let Some(runtime) = update.runtime_kind {
            conn.execute(
                "UPDATE app_sessions SET runtime_kind = ?1, last_active_at = ?2 WHERE id = ?3",
                params![runtime, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn delete_app_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "UPDATE app_sessions SET closed_at = ?1 WHERE id = ?2",
            params![now, &id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn reopen_app_session(state: State<'_, AppState>, id: String) -> Result<AppSession, String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "UPDATE app_sessions SET closed_at = NULL, last_active_at = ?1 WHERE id = ?2",
            params![now, &id],
        )
        .map_err(|e| e.to_string())?;
        let session = conn
            .query_row(
                "SELECT id, title, active_role, runtime_kind, cwd, created_at, last_active_at, closed_at
                 FROM app_sessions WHERE id = ?1",
                params![&id],
                |row| {
                    Ok(AppSession {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        active_role: row.get(2)?,
                        runtime_kind: row.get(3)?,
                        cwd: row.get(4)?,
                        messages: Vec::new(),
                        created_at: row.get(5)?,
                        last_active_at: row.get(6)?,
                        closed_at: row.get(7)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(session)
    })
}
