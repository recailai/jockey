use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::{AppState, ImageAttachment, InboxMessage};
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

fn decode_attachments(raw: String) -> Vec<ImageAttachment> {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn query_inbox(
    conn: &rusqlite::Connection,
    session_id: &str,
    include_claimed: bool,
) -> Result<Vec<InboxMessage>, String> {
    let sql = if include_claimed {
        "SELECT id, app_session_id, role_name, delivery, text, attachments_json, created_at
         FROM session_inbox_messages WHERE app_session_id = ?1 ORDER BY created_at ASC, id ASC"
    } else {
        "SELECT id, app_session_id, role_name, delivery, text, attachments_json, created_at
         FROM session_inbox_messages WHERE app_session_id = ?1 AND claimed_at IS NULL
         ORDER BY created_at ASC, id ASC"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |row| {
            let attachments: String = row.get(5)?;
            Ok(InboxMessage {
                id: row.get(0)?,
                app_session_id: row.get(1)?,
                role_name: row.get(2)?,
                delivery: row.get(3)?,
                text: row.get(4)?,
                attachments: decode_attachments(attachments),
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| row.map_err(|e| e.to_string())).collect()
}

pub(crate) fn list_inbox_internal(
    state: &AppState,
    session_id: &str,
    include_claimed: bool,
) -> Result<Vec<InboxMessage>, String> {
    with_db(state, |conn| query_inbox(conn, session_id, include_claimed))
}

pub(crate) fn enqueue_inbox_internal(
    state: &AppState,
    session_id: &str,
    role_name: Option<&str>,
    delivery: &str,
    text: &str,
    attachments: &[ImageAttachment],
) -> Result<InboxMessage, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("inbox message cannot be empty".to_string());
    }
    if !matches!(delivery, "nextTurn" | "nextStep") {
        return Err("unsupported inbox delivery mode".to_string());
    }
    let id = Uuid::new_v4().to_string();
    let created_at = now_ms();
    let attachments_json = serde_json::to_string(attachments).map_err(|e| e.to_string())?;
    with_db(state, |conn| {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM app_sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("session not found: {session_id}"));
        }
        conn.execute(
            "INSERT INTO session_inbox_messages
             (id, app_session_id, role_name, delivery, text, attachments_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                session_id,
                role_name,
                delivery,
                text,
                attachments_json,
                created_at
            ],
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(InboxMessage {
        id,
        app_session_id: session_id.to_string(),
        role_name: role_name.map(str::to_string),
        delivery: delivery.to_string(),
        text: text.to_string(),
        attachments: attachments.to_vec(),
        created_at,
    })
}

pub(crate) fn remove_inbox_internal(state: &AppState, id: &str) -> Result<(), String> {
    with_db(state, |conn| {
        conn.execute(
            "DELETE FROM session_inbox_messages WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn claim_inbox_internal(state: &AppState, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let claimed_at = now_ms();
    with_db(state, |conn| {
        let mut claimed_ids = Vec::with_capacity(ids.len());
        for id in ids {
            let changed = conn
                .execute(
                    "UPDATE session_inbox_messages
                     SET claimed_at = ?1
                     WHERE id = ?2 AND claimed_at IS NULL",
                    params![claimed_at, id],
                )
                .map_err(|e| e.to_string())?;
            if changed != 1 {
                for claimed_id in &claimed_ids {
                    let _ = conn.execute(
                        "UPDATE session_inbox_messages SET claimed_at = NULL WHERE id = ?1",
                        params![claimed_id],
                    );
                }
                return Err(format!("inbox item is no longer available: {id}"));
            }
            claimed_ids.push(id);
        }
        Ok(())
    })
}

pub(crate) fn restore_inbox_internal(state: &AppState, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    with_db(state, |conn| {
        for id in ids {
            conn.execute(
                "UPDATE session_inbox_messages SET claimed_at = NULL WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn list_session_inbox_cmd(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<InboxMessage>, String> {
    list_inbox_internal(get_state(&state), &session_id, false)
}

#[tauri::command]
pub(crate) fn enqueue_session_inbox_cmd(
    state: State<'_, AppState>,
    session_id: String,
    role_name: Option<String>,
    delivery: Option<String>,
    text: String,
    attachments: Option<Vec<ImageAttachment>>,
) -> Result<InboxMessage, String> {
    enqueue_inbox_internal(
        get_state(&state),
        &session_id,
        role_name.as_deref(),
        delivery.as_deref().unwrap_or("nextTurn"),
        &text,
        attachments.as_deref().unwrap_or(&[]),
    )
}

#[tauri::command]
pub(crate) fn remove_session_inbox_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    remove_inbox_internal(get_state(&state), &id)
}

#[tauri::command]
pub(crate) fn claim_session_inbox_cmd(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    claim_inbox_internal(get_state(&state), &ids)
}

#[tauri::command]
pub(crate) fn restore_session_inbox_cmd(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    restore_inbox_internal(get_state(&state), &ids)
}
