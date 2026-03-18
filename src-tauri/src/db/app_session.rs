use crate::db::{ensure_default_team_id, get_state, with_db};
use crate::now_ms;
use crate::runtime_kind::RuntimeKind;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

fn canonical_runtime_key(runtime_key: &str) -> String {
    RuntimeKind::from_str(runtime_key)
        .map(|k| k.runtime_key().to_string())
        .unwrap_or_else(|| runtime_key.trim().to_ascii_lowercase())
}

fn role_runtime_key(runtime_key: &str, role_name: &str) -> String {
    format!("{}:{role_name}", canonical_runtime_key(runtime_key))
}

fn parse_role_sessions_map(raw: Option<String>) -> serde_json::Map<String, Value> {
    raw.and_then(|v| serde_json::from_str::<serde_json::Map<String, Value>>(&v).ok())
        .unwrap_or_default()
}

pub(crate) fn cleanup_legacy_role_session_mappings(state: &AppState) -> Result<usize, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, role_sessions_json FROM app_sessions")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut updated = 0usize;
        for row in rows {
            let (session_id, raw_json) = match row {
                Ok(v) => v,
                Err(_) => continue,
            };
            let parsed = parse_role_sessions_map(Some(raw_json.clone()));
            let mut cleaned = serde_json::Map::<String, Value>::new();
            for (key, value) in parsed {
                let Some((runtime, role_name)) = key.split_once(':') else {
                    continue;
                };
                let Some(session_id_value) = value.as_str() else {
                    continue;
                };
                let normalized_runtime = canonical_runtime_key(runtime);
                if !KNOWN_RUNTIME_KEYS.contains(&normalized_runtime.as_str()) {
                    continue;
                }
                cleaned.insert(
                    format!("{normalized_runtime}:{role_name}"),
                    Value::String(session_id_value.to_string()),
                );
            }
            let cleaned_json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
            if cleaned_json != raw_json {
                conn.execute(
                    "UPDATE app_sessions SET role_sessions_json = ?1 WHERE id = ?2",
                    params![cleaned_json, session_id],
                )
                .map_err(|e| e.to_string())?;
                updated += 1;
            }
        }
        Ok(updated)
    })
}

pub(crate) fn load_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
) -> Option<String> {
    if app_session_id.is_empty() {
        return None;
    }
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
    .and_then(|map| {
        let role_key = role_runtime_key(runtime_key, role_name);
        let parsed = parse_role_sessions_map(Some(map));
        parsed
            .get(&role_key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

pub(crate) fn save_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
    cli_session_id: &str,
) -> Result<(), String> {
    if app_session_id.is_empty() {
        return Ok(());
    }
    with_db(state, |conn| {
        let existing = conn
            .query_row(
                "SELECT role_sessions_json FROM app_sessions WHERE id = ?1",
                params![app_session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut role_sessions = parse_role_sessions_map(existing);
        let normalized_runtime = canonical_runtime_key(runtime_key);
        role_sessions.retain(|key, _| {
            if key == role_name {
                return false;
            }
            if let Some((stored_runtime, stored_role)) = key.split_once(':') {
                return !(stored_role == role_name
                    && canonical_runtime_key(stored_runtime) == normalized_runtime);
            }
            true
        });
        role_sessions.insert(
            format!("{normalized_runtime}:{role_name}"),
            Value::String(cli_session_id.to_string()),
        );
        let encoded = serde_json::to_string(&role_sessions).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE app_sessions SET role_sessions_json = ?1 WHERE id = ?2",
            params![encoded, app_session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn list_app_sessions(state: State<'_, AppState>) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, team_id, active_role, selected_assistant, created_at, last_active_at
                 FROM app_sessions ORDER BY last_active_at DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let sessions: Vec<(String, AppSession)> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                Ok((
                    id.clone(),
                    AppSession {
                        id,
                        title: row.get(1)?,
                        active_role: row.get(3)?,
                        selected_assistant: row.get(4)?,
                        messages: Vec::new(),
                        created_at: row.get(5)?,
                        last_active_at: row.get(6)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut msg_stmt = conn
            .prepare(
                "SELECT session_id, role, role_label, content, created_at
                 FROM app_session_messages
                 WHERE session_id IN (SELECT id FROM app_sessions ORDER BY last_active_at DESC LIMIT 50)
                 ORDER BY session_id, id ASC",
            )
            .map_err(|e| e.to_string())?;
        let mut msgs_by_session: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for row in msg_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
        {
            let (session_id, role, role_label, content, at) = row;
            let mut v = serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "role": role,
                "text": content,
                "at": at
            });
            if let Some(label) = role_label {
                v["roleLabel"] = serde_json::Value::String(label);
            }
            msgs_by_session.entry(session_id).or_default().push(v);
        }

        let mut out = Vec::with_capacity(sessions.len());
        for (session_id, mut session) in sessions {
            session.messages = msgs_by_session.remove(&session_id).unwrap_or_default();
            out.push(session);
        }
        Ok(out)
    })
}

#[tauri::command]
pub(crate) fn append_app_message(
    state: State<'_, AppState>,
    session_id: String,
    role: String,
    role_label: Option<String>,
    content: String,
) -> Result<(), String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO app_session_messages (session_id, role, role_label, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&session_id, &role, &role_label, &content, now],
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
    let team_id = ensure_default_team_id(get_state(&state))?;
    let now = now_ms();
    let session = AppSession {
        id: Uuid::new_v4().to_string(),
        title: title.unwrap_or_else(|| "New Session".to_string()),
        active_role: "UnionAI".to_string(),
        selected_assistant: None,
        messages: Vec::new(),
        created_at: now,
        last_active_at: now,
    };
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO app_sessions (id, title, team_id, active_role, selected_assistant, created_at, last_active_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &session.id,
                &session.title,
                &team_id,
                &session.active_role,
                &session.selected_assistant,
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
        if let Some(assistant) = update.selected_assistant {
            conn.execute(
                "UPDATE app_sessions SET selected_assistant = ?1, last_active_at = ?2 WHERE id = ?3",
                params![assistant, now, &id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn delete_app_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_db(get_state(&state), |conn| {
        conn.execute(
            "DELETE FROM app_session_messages WHERE session_id = ?1",
            params![&id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM app_sessions WHERE id = ?1", params![&id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn cleanup_legacy_role_sessions(state: State<'_, AppState>) -> Result<usize, String> {
    cleanup_legacy_role_session_mappings(get_state(&state))
}
