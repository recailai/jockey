use crate::acp;
use crate::db::{get_state, in_tx, with_db};
use crate::runtime_profile;
use crate::types::*;
use crate::{default_chat_cwd, now_ms};
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

fn validate_session_title(raw: &str) -> Result<String, String> {
    let title = raw.trim().to_string();
    if title.is_empty() {
        return Err("session name required".to_string());
    }
    if title.chars().any(|c| c.is_whitespace()) {
        return Err("session name cannot contain spaces".to_string());
    }
    Ok(title)
}

fn active_session_title_exists(
    conn: &rusqlite::Connection,
    title: &str,
    project_id: Option<&str>,
    exclude_id: Option<&str>,
) -> Result<bool, String> {
    let exists = match (project_id, exclude_id) {
        (Some(pid), Some(eid)) => conn
            .query_row(
                "SELECT 1 FROM app_sessions WHERE lower(title) = lower(?1) AND closed_at IS NULL AND project_id = ?2 AND id <> ?3 LIMIT 1",
                params![title, pid, eid],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some(),
        (Some(pid), None) => conn
            .query_row(
                "SELECT 1 FROM app_sessions WHERE lower(title) = lower(?1) AND closed_at IS NULL AND project_id = ?2 LIMIT 1",
                params![title, pid],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some(),
        (None, Some(eid)) => conn
            .query_row(
                "SELECT 1 FROM app_sessions WHERE lower(title) = lower(?1) AND closed_at IS NULL AND project_id IS NULL AND id <> ?2 LIMIT 1",
                params![title, eid],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some(),
        (None, None) => conn
            .query_row(
                "SELECT 1 FROM app_sessions WHERE lower(title) = lower(?1) AND closed_at IS NULL AND project_id IS NULL LIMIT 1",
                params![title],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some(),
    };
    Ok(exists)
}

/// The columns the engine needs to route a turn, without loading the transcript.
pub(crate) struct SessionHead {
    pub(crate) title: String,
    pub(crate) active_role: String,
    pub(crate) runtime_kind: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) closed: bool,
}

pub(crate) fn load_session_head(
    state: &AppState,
    session_id: &str,
) -> Result<Option<SessionHead>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT title, active_role, runtime_kind, project_id, closed_at
             FROM app_sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok(SessionHead {
                    title: row.get(0)?,
                    active_role: row.get(1)?,
                    runtime_kind: row.get(2)?,
                    project_id: row.get(3)?,
                    closed: row.get::<_, Option<i64>>(4)?.is_some(),
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

const PREVIEW_CHARS: usize = 120;

fn compact_preview(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(PREVIEW_CHARS) {
        Some((idx, _)) => format!("{}…", &compact[..idx]),
        None => compact,
    }
}

/// Sidebar rows: message count and last non-event preview come from indexed subqueries on
/// `app_session_messages(session_id, id)`, so listing never loads transcripts.
pub(crate) fn list_session_summaries(
    state: &AppState,
    project_id: Option<&str>,
    closed: bool,
    ids: Option<&[String]>,
) -> Result<Vec<crate::types::SessionSummary>, String> {
    let mut sql = String::from(
        "SELECT s.id, s.title, s.active_role, s.runtime_kind, s.project_id, s.last_active_at, s.closed_at, s.cwd,
                (SELECT count(1) FROM app_session_messages m WHERE m.session_id = s.id),
                (SELECT substr(m.content, 1, 400) FROM app_session_messages m
                  WHERE m.session_id = s.id AND m.role_name <> 'event' AND trim(m.content) <> ''
                  ORDER BY m.id DESC LIMIT 1),
                (SELECT CASE WHEN a.state = 'error' THEN a.last_error END
                   FROM agent_lifecycle a WHERE a.app_session_id = s.id
                   ORDER BY a.updated_at DESC LIMIT 1)
         FROM app_sessions s WHERE 1 = 1",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sql.push_str(&format!(
            " AND s.id IN ({})",
            std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(",")
        ));
        args.extend(ids.iter().map(|id| id.clone().into()));
    } else {
        sql.push_str(if closed {
            " AND s.closed_at IS NOT NULL"
        } else {
            " AND s.closed_at IS NULL"
        });
        if let Some(pid) = project_id.filter(|p| !p.trim().is_empty()) {
            sql.push_str(" AND s.project_id = ?");
            args.push(pid.to_string().into());
        }
    }
    sql.push_str(if closed {
        " ORDER BY s.closed_at DESC LIMIT 200"
    } else {
        " ORDER BY s.last_active_at DESC LIMIT 200"
    });
    with_db(state, |conn| {
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                Ok(crate::types::SessionSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    active_role: row.get(2)?,
                    runtime_kind: row.get(3)?,
                    project_id: row.get(4)?,
                    last_active_at: row.get(5)?,
                    closed_at: row.get(6)?,
                    cwd: row.get(7)?,
                    message_count: row.get::<_, i64>(8)? as usize,
                    preview: row
                        .get::<_, Option<String>>(9)?
                        .map(|t| compact_preview(&t)),
                    running: false,
                    last_error: row.get(10)?,
                    external_active: false,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
}

pub(crate) fn delete_messages_by_client_ids(
    state: &AppState,
    session_id: &str,
    client_ids: &[String],
) -> Result<(), String> {
    if client_ids.is_empty() {
        return Ok(());
    }
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare("DELETE FROM app_session_messages WHERE session_id = ?1 AND client_id = ?2")
            .map_err(|e| e.to_string())?;
        for cid in client_ids {
            stmt.execute(params![session_id, cid])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

pub(crate) fn count_user_messages(state: &AppState, session_id: &str) -> Result<i64, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT count(1) FROM app_session_messages WHERE session_id = ?1 AND role_name = 'user'",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    })
}

/// Rename to `desired`, or `desired_copy`, `desired_copy2`, … when an open session in the same
/// project already has that name — the rule the desktop's `uniqueName` applied.
pub(crate) fn rename_app_session_unique(
    state: &AppState,
    session_id: &str,
    desired: &str,
) -> Result<String, String> {
    let desired = validate_session_title(desired)?;
    with_db(state, |conn| {
        in_tx(conn, |conn| {
            let project_id: Option<String> = conn
                .query_row(
                    "SELECT project_id FROM app_sessions WHERE id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let taken = |title: &str| {
                active_session_title_exists(conn, title, project_id.as_deref(), Some(session_id))
            };
            let mut title = desired.clone();
            if taken(&title)? {
                let base = desired
                    .rsplit_once("_copy")
                    .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()))
                    .map(|(b, _)| b.to_string())
                    .unwrap_or_else(|| desired.clone());
                title = format!("{base}_copy");
                let mut n = 2;
                while taken(&title)? {
                    title = format!("{base}_copy{n}");
                    n += 1;
                }
            }
            conn.execute(
                "UPDATE app_sessions SET title = ?1 WHERE id = ?2",
                params![&title, session_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(title)
        })
    })
}

/// Persist the persona (and its engine) a turn was routed to, so every client sees the switch.
pub(crate) fn set_app_session_route(
    state: &AppState,
    session_id: &str,
    active_role: &str,
    runtime_kind: Option<&str>,
    runtime_profile_id: Option<&str>,
) -> Result<(), String> {
    with_db(state, |conn| {
        match runtime_kind {
            Some(rt) => conn.execute(
                "UPDATE app_sessions SET active_role = ?1, runtime_kind = ?2, runtime_profile_id = ?3 WHERE id = ?4",
                params![active_role, rt, runtime_profile_id, session_id],
            ),
            None => conn.execute(
                "UPDATE app_sessions SET active_role = ?1 WHERE id = ?2",
                params![active_role, session_id],
            ),
        }
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

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

pub(crate) fn get_app_session_project_id(state: &AppState, session_id: &str) -> Option<String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT project_id FROM app_sessions WHERE id = ?1",
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

fn query_sessions(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<AppSession>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mut sessions: Vec<AppSession> = stmt
        .query_map(params, |row| {
            Ok(AppSession {
                id: row.get(0)?,
                title: row.get(1)?,
                active_role: row.get(2)?,
                runtime_kind: row.get(3)?,
                runtime_profile_id: row.get(4)?,
                cwd: row.get(5)?,
                project_id: row.get(6)?,
                external_session_id: row.get(7)?,
                messages: Vec::new(),
                created_at: row.get(8)?,
                last_active_at: row.get(9)?,
                closed_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if sessions.is_empty() {
        return Ok(sessions);
    }

    // One query for every session's messages rather than one per session: a session list
    // returns up to 100 rows, and the per-session version made this 101 round trips.
    let placeholders = std::iter::repeat_n("?", sessions.len())
        .collect::<Vec<_>>()
        .join(",");
    let msg_sql = format!(
        "SELECT session_id, id, role_name, content, content_type, payload, created_at, client_id
         FROM app_session_messages
         WHERE session_id IN ({placeholders})
         ORDER BY id ASC"
    );
    let msg_params: Vec<&dyn rusqlite::ToSql> = sessions
        .iter()
        .map(|s| &s.id as &dyn rusqlite::ToSql)
        .collect();

    let mut msg_stmt = conn.prepare(&msg_sql).map_err(|e| e.to_string())?;
    let rows = msg_stmt
        .query_map(msg_params.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut by_session: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for row in rows {
        let (session_id, message_id, role_name, content, content_type, payload, at, client_id) =
            row.map_err(|e| e.to_string())?;
        let id_val = client_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("m{message_id}"));
        let mut obj = serde_json::json!({
            // Use client_id if available so IDs are stable and match the frontend across reloads;
            // falls back to the table's own autoincrement key.
            "id": id_val,
            "roleName": role_name,
            "text": content,
            "at": at
        });
        // Structured extras (toolCalls/segments/images/thoughtText) merge over the plain
        // base — a corrupt payload just falls back to the plain-text object rather than
        // failing the whole session load, same defensive stance as `parse_payload`.
        if content_type == "json" {
            if let Some(raw) = payload {
                if let Ok(serde_json::Value::Object(mut extra)) =
                    serde_json::from_str::<serde_json::Value>(&raw)
                {
                    // Do not allow payload to override the resolved message id
                    extra.remove("id");
                    if let serde_json::Value::Object(base) = &mut obj {
                        for (k, v) in extra {
                            base.insert(k, v);
                        }
                    }
                }
            }
        }
        by_session.entry(session_id).or_default().push(obj);
    }

    // `ORDER BY id ASC` is global, and the key is monotonic per insert, so each session's
    // slice comes out in insertion order without a second sort.
    for session in sessions.iter_mut() {
        if let Some(messages) = by_session.remove(&session.id) {
            session.messages = messages;
        }
    }

    Ok(sessions)
}

/// Load specific sessions with their messages attached. The import path needs this because
/// `upsert_imported_session` returns the row it just wrote, before transcript messages exist.
pub(crate) fn get_app_sessions_by_ids(
    state: &AppState,
    ids: &[String],
) -> Result<Vec<AppSession>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id,
                external_session_id, created_at, last_active_at, closed_at
         FROM app_sessions WHERE id IN ({placeholders}) ORDER BY last_active_at DESC"
    );
    with_db(state, |conn| {
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        query_sessions(conn, &sql, &params)
    })
}

// Loads transcripts: keep it off the main thread, which would freeze the window.
#[tauri::command(async)]
pub(crate) fn list_app_sessions(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        if let Some(pid) = project_id.as_deref().filter(|s| !s.trim().is_empty()) {
            query_sessions(
                conn,
                "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at
                 FROM app_sessions WHERE closed_at IS NULL AND project_id = ?1 ORDER BY last_active_at DESC LIMIT 100",
                &[&pid],
            )
        } else {
            query_sessions(
                conn,
                "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at
                 FROM app_sessions WHERE closed_at IS NULL ORDER BY last_active_at DESC LIMIT 100",
                &[],
            )
        }
    })
}

// Loads transcripts: keep it off the main thread, which would freeze the window.
#[tauri::command(async)]
pub(crate) fn list_closed_app_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<AppSession>, String> {
    with_db(get_state(&state), |conn| {
        query_sessions(
            conn,
            "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at
             FROM app_sessions WHERE closed_at IS NOT NULL ORDER BY closed_at DESC LIMIT 200",
            &[],
        )
    })
}

pub(crate) fn save_app_message_internal(
    state: &AppState,
    session_id: &str,
    client_id: Option<&str>,
    role_name: &str,
    content: &str,
    content_type: Option<&str>,
    payload: Option<&str>,
) -> Result<(), String> {
    let now = now_ms();
    let content_type = content_type
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("text");
    let client_id = client_id.map(str::trim).filter(|s| !s.is_empty());
    with_db(state, |conn| {
        // One statement, so two writers for the same client id cannot both insert
        // (uq_app_session_messages_session_client_id). Rows without a client id always append.
        conn.execute(
            "INSERT INTO app_session_messages (session_id, client_id, role_name, content, content_type, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(session_id, client_id) WHERE client_id IS NOT NULL DO UPDATE SET
               role_name = excluded.role_name,
               content = excluded.content,
               content_type = excluded.content_type,
               payload = excluded.payload",
            params![session_id, client_id, role_name, content, content_type, payload, now],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE app_sessions SET last_active_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn append_app_message_internal(
    state: &AppState,
    session_id: &str,
    role_name: &str,
    content: &str,
    content_type: Option<&str>,
    payload: Option<&str>,
) -> Result<(), String> {
    save_app_message_internal(
        state,
        session_id,
        None,
        role_name,
        content,
        content_type,
        payload,
    )
}

#[tauri::command]
pub(crate) fn append_app_message(
    state: State<'_, AppState>,
    session_id: String,
    role_name: String,
    content: String,
    content_type: Option<String>,
    payload: Option<String>,
) -> Result<(), String> {
    append_app_message_internal(
        get_state(&state),
        &session_id,
        &role_name,
        &content,
        content_type.as_deref(),
        payload.as_deref(),
    )
}

#[tauri::command]
pub(crate) fn save_app_message(
    state: State<'_, AppState>,
    session_id: String,
    client_id: Option<String>,
    role_name: String,
    content: String,
    content_type: Option<String>,
    payload: Option<String>,
) -> Result<(), String> {
    save_app_message_internal(
        get_state(&state),
        &session_id,
        client_id.as_deref(),
        &role_name,
        &content,
        content_type.as_deref(),
        payload.as_deref(),
    )
}

/// Messages a session holds: `(all, imported-with-ids)`. A session imported before transcript
/// rows carried ids has messages but none with the import prefix.
pub(crate) fn imported_message_counts(
    state: &AppState,
    session_id: &str,
    id_prefix: &str,
) -> Result<(i64, i64), String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT count(1), coalesce(sum(client_id LIKE ?2 || ':%'), 0)
             FROM app_session_messages WHERE session_id = ?1",
            params![session_id, id_prefix],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())
    })
}

/// One transcript item to store: a user prompt, an event row, or a whole assistant turn
/// (`content_type = json` with the same payload the engine writes for its own turns).
#[derive(Clone)]
pub(crate) struct ImportRow {
    pub(crate) client_id: String,
    pub(crate) role_name: String,
    pub(crate) content: String,
    pub(crate) content_type: Option<&'static str>,
    pub(crate) payload: Option<String>,
    pub(crate) created_at: i64,
}

/// Store imported rows under their provider-native ids. Rows already present are left alone
/// (the unique index makes a repeated or overlapping import a no-op); returns, per input row,
/// whether it was new. `replace_legacy_prefix` first deletes the positional rows
/// (`{prefix}:{index}`) an older importer wrote, in the same transaction, so upgrading a
/// pure import to turn-shaped rows never shows both.
pub(crate) fn insert_import_rows(
    state: &AppState,
    session_id: &str,
    rows: &[ImportRow],
    replace_legacy_prefix: Option<&str>,
) -> Result<Vec<bool>, String> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    with_db(state, |conn| {
        in_tx(conn, |conn| {
            if let Some(prefix) = replace_legacy_prefix {
                conn.execute(
                    "DELETE FROM app_session_messages
                     WHERE session_id = ?1 AND client_id LIKE ?2 || ':%'
                       AND substr(client_id, length(?2) + 2) <> ''
                       AND substr(client_id, length(?2) + 2) NOT GLOB '*[^0-9]*'",
                    params![session_id, prefix],
                )
                .map_err(|e| e.to_string())?;
            }
            let mut stmt = conn
                .prepare(
                    "INSERT INTO app_session_messages
                       (session_id, client_id, role_name, content, content_type, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(session_id, client_id) WHERE client_id IS NOT NULL DO NOTHING",
                )
                .map_err(|e| e.to_string())?;
            let mut added = Vec::with_capacity(rows.len());
            let mut newest = 0i64;
            for row in rows {
                let inserted = stmt
                    .execute(params![
                        session_id,
                        row.client_id,
                        row.role_name,
                        row.content,
                        row.content_type.unwrap_or("text"),
                        row.payload,
                        row.created_at
                    ])
                    .map_err(|e| e.to_string())?;
                if inserted > 0 {
                    newest = newest.max(row.created_at);
                }
                added.push(inserted > 0);
            }
            drop(stmt);
            if newest > 0 {
                // A session that moved in the terminal moves up the list; never down.
                conn.execute(
                    "UPDATE app_sessions SET last_active_at = max(last_active_at, ?1) WHERE id = ?2",
                    params![newest, session_id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(added)
        })
    })
}

/// The latest moment the session is known to have recorded anything: its newest message or
/// its `last_active_at`, whichever is later. A Jockey turn's row keeps the time of its first
/// checkpoint while the turn runs on, but every save bumps `last_active_at`, so the pair
/// bounds the turn's end.
pub(crate) fn newest_message_at(state: &AppState, session_id: &str) -> Result<i64, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT max(
               coalesce((SELECT max(created_at) FROM app_session_messages WHERE session_id = ?1), 0),
               coalesce((SELECT last_active_at FROM app_sessions WHERE id = ?1), 0))",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    })
}

/// `base`, or `base_2`, `base_3`, ... when an open session in the project already has the title.
fn unique_title(
    conn: &rusqlite::Connection,
    base: &str,
    project_id: Option<&str>,
) -> Result<String, String> {
    let mut title = base.to_string();
    let mut counter = 1;
    while active_session_title_exists(conn, &title, project_id, None)? {
        counter += 1;
        title = format!("{base}_{counter}");
    }
    Ok(title)
}

pub(crate) fn create_app_session_internal(
    state: &AppState,
    title: Option<&str>,
    project_id: Option<&str>,
    runtime_kind: Option<&str>,
    runtime_profile_id: Option<&str>,
    id: Option<&str>,
) -> Result<AppSession, String> {
    let now = now_ms();
    let base_title = validate_session_title(title.unwrap_or("Session_1"))?;
    let id = id.map(str::trim).filter(|s| !s.is_empty());

    let project_id = project_id.map(str::trim).filter(|id| !id.is_empty());
    let cwd = if let Some(pid) = project_id {
        let project = crate::db::project::get_project_internal(state, pid)?
            .ok_or_else(|| format!("project not found: {pid}"))?;
        Some(project.root_path)
    } else {
        Some(default_chat_cwd())
    };

    with_db(state, |conn| {
        in_tx(conn, |conn| {
            let final_title = unique_title(conn, &base_title, project_id)?;

            let session = AppSession {
                id: id
                    .map(str::to_string)
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                title: final_title,
                active_role: "Developer".to_string(),
                runtime_kind: runtime_kind.map(|s| s.to_string()),
                runtime_profile_id: runtime_profile_id.map(|s| s.to_string()),
                cwd,
                project_id: project_id.map(|s| s.to_string()),
                external_session_id: None,
                messages: Vec::new(),
                created_at: now,
                last_active_at: now,
                closed_at: None,
            };

            conn.execute(
            "INSERT INTO app_sessions (id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                &session.id,
                &session.title,
                &session.active_role,
                &session.runtime_kind,
                &session.runtime_profile_id,
                &session.cwd,
                &session.project_id,
                &session.external_session_id,
                session.created_at,
                session.last_active_at,
            ],
        )
        .map_err(|e| e.to_string())?;
            Ok(session)
        })
    })
}

/// A new open session beside `source_id` (same project, cwd, persona and engine) holding a copy
/// of its transcript, in one transaction. `title` must already be a single token; a clash with
/// an open session gets a numeric suffix like any other new session.
///
/// `provider` is the forked provider conversation `(runtime_key, role, handle)`. It is bound in
/// the same transaction, together with the source's model/mode/option overrides for that
/// binding, so a failure leaves no session that would silently start a fresh conversation.
/// Handles of the source's other personas are not carried over: only the active persona was
/// forked at the provider, so the others stay unbound.
pub(crate) fn fork_app_session_internal(
    state: &AppState,
    source_id: &str,
    title: &str,
    provider: Option<(&str, &str, &str)>,
) -> Result<AppSession, String> {
    let now = now_ms();
    with_db(state, |conn| {
        in_tx(conn, |conn| {
            let (active_role, runtime_kind, runtime_profile_id, cwd, project_id): (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = conn
                .query_row(
                    "SELECT active_role, runtime_kind, runtime_profile_id, cwd, project_id
                     FROM app_sessions WHERE id = ?1",
                    params![source_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "session not found".to_string())?;
            let final_title = unique_title(conn, title, project_id.as_deref())?;
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO app_sessions (id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8, NULL)",
                params![
                    id,
                    final_title,
                    active_role,
                    runtime_kind,
                    runtime_profile_id,
                    cwd,
                    project_id,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO app_session_messages (session_id, client_id, role_name, content, content_type, payload, created_at)
                 SELECT ?1, client_id, role_name, content, content_type, payload, created_at
                 FROM app_session_messages WHERE session_id = ?2 ORDER BY id",
                params![id, source_id],
            )
            .map_err(|e| e.to_string())?;
            if let Some((runtime_key, role, handle)) = provider {
                if handle.trim().is_empty() {
                    return Err("The agent returned no session id for the fork".to_string());
                }
                crate::db::app_session_role::save_cli_id_on(conn, &id, runtime_key, role, handle)?;
                crate::db::app_session_role::copy_role_overrides_on(
                    conn,
                    source_id,
                    &id,
                    role,
                    runtime_key,
                )?;
            }
            Ok(AppSession {
                id,
                title: final_title,
                active_role,
                runtime_kind,
                runtime_profile_id,
                cwd,
                project_id,
                external_session_id: None,
                messages: Vec::new(),
                created_at: now,
                last_active_at: now,
                closed_at: None,
            })
        })
    })
}

pub(crate) fn upsert_imported_session(
    state: &AppState,
    project_id: Option<&str>,
    title: &str,
    cwd: &str,
    runtime_kind: &str,
    external_session_id: &str,
    created_at: i64,
    last_active_at: i64,
) -> Result<AppSession, String> {
    with_db(state, |conn| {
        in_tx(conn, |conn| {
            crate::db::project::ensure_project_exists(conn, project_id)?;
            let existing_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM app_sessions
                 WHERE external_session_id = ?1 AND runtime_kind = ?2
                 LIMIT 1",
                    params![external_session_id, runtime_kind],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;

            if let Some(id) = existing_id {
                // Keep the later of the two: the session may have been used in Jockey after the
                // transcript was last written, and must not drop down the list on a re-import.
                conn.execute(
                "UPDATE app_sessions SET last_active_at = max(last_active_at, ?1), project_id = coalesce(project_id, ?2), cwd = ?3 WHERE id = ?4",
                params![last_active_at, project_id, cwd, id],
            )
            .map_err(|e| e.to_string())?;

                return Ok(AppSession {
                    id,
                    title: title.to_string(),
                    active_role: "Developer".to_string(),
                    runtime_kind: Some(runtime_kind.to_string()),
                    runtime_profile_id: Some(runtime_profile::profile_id(runtime_kind)),
                    cwd: Some(cwd.to_string()),
                    project_id: project_id.map(|s| s.to_string()),
                    external_session_id: Some(external_session_id.to_string()),
                    messages: Vec::new(),
                    created_at,
                    last_active_at,
                    closed_at: None,
                });
            }

            let id = Uuid::new_v4().to_string();
            // Any whitespace, not just spaces: a tab or newline would fail title validation the
            // first time the user renames the session.
            let mut session_title = title.split_whitespace().collect::<Vec<_>>().join("_");
            if session_title.is_empty() {
                session_title = format!(
                    "Claude_{}",
                    &external_session_id[..8.min(external_session_id.len())]
                );
            }
            let mut final_title = session_title.clone();
            let mut counter = 1;
            while active_session_title_exists(conn, &final_title, project_id, None)? {
                counter += 1;
                final_title = format!("{}_{}", session_title, counter);
            }

            conn.execute(
            "INSERT INTO app_sessions (id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at)
             VALUES (?1, ?2, 'Developer', ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                id,
                final_title,
                runtime_kind,
                runtime_profile::profile_id(runtime_kind),
                cwd,
                project_id,
                external_session_id,
                created_at,
                last_active_at,
            ],
        )
        .map_err(|e| e.to_string())?;

            Ok(AppSession {
                id,
                title: final_title,
                active_role: "Developer".to_string(),
                runtime_kind: Some(runtime_kind.to_string()),
                runtime_profile_id: Some(runtime_profile::profile_id(runtime_kind)),
                cwd: Some(cwd.to_string()),
                project_id: project_id.map(|s| s.to_string()),
                external_session_id: Some(external_session_id.to_string()),
                messages: Vec::new(),
                created_at,
                last_active_at,
                closed_at: None,
            })
        })
    })
}

/// Close a session. Returns `false` when it was already closed (or never existed), so a
/// caller can decide whether that is an error; the desktop treats it as a no-op.
pub(crate) fn close_app_session_internal(state: &AppState, id: &str) -> Result<bool, String> {
    let now = now_ms();
    let changed = with_db(state, |conn| {
        conn.execute(
            "UPDATE app_sessions SET closed_at = ?1 WHERE id = ?2 AND closed_at IS NULL",
            params![now, id],
        )
        .map_err(|e| e.to_string())
    })?;
    if changed > 0 {
        schedule_session_role_cleanup(state.clone_refs(), id.to_string(), false);
    }
    Ok(changed > 0)
}

fn schedule_session_role_cleanup(state: AppState, session_id: String, keep_active_role: bool) {
    tauri::async_runtime::spawn(async move {
        let active_role = if keep_active_role {
            with_db(&state, |conn| {
                conn.query_row(
                    "SELECT active_role FROM app_sessions WHERE id = ?1",
                    params![&session_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())
            })
            .ok()
            .flatten()
        } else {
            None
        };

        let targets: Vec<(String, String)> = with_db(&state, |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT role_name, runtime_kind
                     FROM app_session_roles
                     WHERE app_session_id = ?1
                       AND runtime_kind IS NOT NULL
                       AND trim(runtime_kind) <> ''",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![&session_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        })
        .unwrap_or_default();

        for (role_name, runtime_kind) in targets {
            if active_role.as_deref() == Some(role_name.as_str()) {
                continue;
            }
            // Only the warm process goes. The provider handle stays: it is what the session
            // resumes from when reopened and what the background sync follows while it is
            // closed. Only an explicit reset (`reset_acp_session`) drops it.
            let _ = acp::reset_session(&runtime_kind, &role_name, Some(&session_id)).await;
        }
    });
}

pub(crate) fn update_app_session_internal(
    state: &AppState,
    id: &str,
    update: AppSessionUpdate,
) -> Result<(), String> {
    let id = id.to_string();
    let role_changed = update.active_role.is_some();
    with_db(state, |conn| {
        in_tx(conn, |conn| {
            let current_project_id: Option<String> = conn
                .query_row(
                    "SELECT project_id FROM app_sessions WHERE id = ?1",
                    params![&id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session not found: {id}"))?;

            if let Some(ref title) = update.title {
                let title = validate_session_title(title)?;
                if active_session_title_exists(
                    conn,
                    &title,
                    current_project_id.as_deref(),
                    Some(&id),
                )? {
                    return Err(format!("session name already exists: {}", title));
                }
                conn.execute(
                    "UPDATE app_sessions SET title = ?1 WHERE id = ?2",
                    params![title, &id],
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(ref role) = update.active_role {
                conn.execute(
                    "UPDATE app_sessions SET active_role = ?1 WHERE id = ?2",
                    params![role, &id],
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(runtime) = update.runtime_kind {
                conn.execute(
                "UPDATE app_sessions SET runtime_kind = ?1, runtime_profile_id = ?2 WHERE id = ?3",
                params![
                    runtime,
                    runtime.as_deref().map(runtime_profile::profile_id),
                    &id
                ],
            )
            .map_err(|e| e.to_string())?;
            }
            if let Some(profile) = update.runtime_profile_id {
                conn.execute(
                    "UPDATE app_sessions SET runtime_profile_id = ?1 WHERE id = ?2",
                    params![profile, &id],
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(cwd) = update.cwd {
                conn.execute(
                    "UPDATE app_sessions SET cwd = ?1 WHERE id = ?2",
                    params![cwd, &id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    })?;
    if role_changed {
        schedule_session_role_cleanup(state.clone_refs(), id, true);
    }
    Ok(())
}

pub(crate) fn reopen_app_session_internal(
    state: &AppState,
    id: &str,
) -> Result<AppSession, String> {
    let id = id.to_string();
    let now = now_ms();
    with_db(state, |conn| {
        let (title, project_id): (String, Option<String>) = conn
            .query_row(
                "SELECT title, project_id FROM app_sessions WHERE id = ?1",
                params![&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if active_session_title_exists(conn, &title, project_id.as_deref(), Some(&id))? {
            return Err(format!("session name already exists: {}", title));
        }
        conn.execute(
            "UPDATE app_sessions SET closed_at = NULL, last_active_at = ?1 WHERE id = ?2",
            params![now, &id],
        )
        .map_err(|e| e.to_string())?;
        crate::db::app_session_role::rebind_from_external(conn, &id)?;
        let session = conn
            .query_row(
                "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, external_session_id, created_at, last_active_at, closed_at
                 FROM app_sessions WHERE id = ?1",
                params![&id],
                |row| {
                    Ok(AppSession {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        active_role: row.get(2)?,
                        runtime_kind: row.get(3)?,
                        runtime_profile_id: row.get(4)?,
                        cwd: row.get(5)?,
                        project_id: row.get(6)?,
                        external_session_id: row.get(7)?,
                        messages: Vec::new(),
                        created_at: row.get(8)?,
                        last_active_at: row.get(9)?,
                        closed_at: row.get(10)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(session)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::db::pool::DbPool;
    use dashmap::DashMap;
    use std::sync::Arc;

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(
            dir.path().join("test.sqlite3"),
            2,
            "PRAGMA foreign_keys = ON;",
        )
        .expect("pool");
        {
            let conn = pool.get().expect("conn");
            init_db(&conn).expect("init_db");
        }
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    #[test]
    fn summaries_report_agent_errors_and_clear_them_after_reconnect() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        let transition = |next, error| {
            crate::db::lifecycle::transition_internal(
                &state,
                &session.id,
                "Developer",
                "codex-cli",
                next,
                error,
            )
            .unwrap();
        };
        transition("running", None);
        transition("error", Some("Sign in required"));
        let rows = list_session_summaries(&state, None, false, None).unwrap();
        assert_eq!(rows[0].last_error.as_deref(), Some("Sign in required"));
        transition("prewarming", None);
        let rows = list_session_summaries(&state, None, false, None).unwrap();
        assert!(rows[0].last_error.is_none());
    }

    fn row(id: &str, at: i64) -> ImportRow {
        ImportRow {
            client_id: id.to_string(),
            role_name: "user".into(),
            content: id.to_string(),
            content_type: None,
            payload: None,
            created_at: at,
        }
    }

    #[test]
    fn reimport_appends_only_rows_with_new_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        let first = [row("import:x:1:u:a", 1), row("import:x:1:t:b", 2)];
        assert_eq!(
            insert_import_rows(&state, &session.id, &first, None).unwrap(),
            [true, true]
        );
        let more = [first[0].clone(), first[1].clone(), row("import:x:1:u:c", 3)];
        assert_eq!(
            insert_import_rows(&state, &session.id, &more, None).unwrap(),
            [false, false, true]
        );
        assert_eq!(
            imported_message_counts(&state, &session.id, "import:x:1").unwrap(),
            (3, 3)
        );
        assert!(newest_message_at(&state, &session.id).unwrap() >= 3);
    }

    #[test]
    fn upgrading_replaces_positional_rows_but_keeps_everything_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        insert_import_rows(
            &state,
            &session.id,
            &[
                row("import:x:1:0", 1),
                row("import:x:1:1", 2),
                row("import:x:1:u:keep", 3),
            ],
            None,
        )
        .unwrap();
        save_app_message_internal(&state, &session.id, Some("own"), "user", "mine", None, None)
            .unwrap();
        insert_import_rows(
            &state,
            &session.id,
            &[row("import:x:1:t:new", 4)],
            Some("import:x:1"),
        )
        .unwrap();
        let ids: Vec<String> = with_db(&state, |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT client_id FROM app_session_messages WHERE session_id = ?1 ORDER BY id",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![&session.id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();
            Ok(rows)
        })
        .unwrap();
        assert_eq!(ids, ["import:x:1:u:keep", "own", "import:x:1:t:new"]);
    }

    #[test]
    fn structured_message_round_trips_through_persist_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");

        let payload = serde_json::json!({
            "text": "ran a tool",
            "toolCalls": [{"toolCallId": "t1", "kind": "edit", "status": "completed"}],
        })
        .to_string();
        append_app_message_internal(
            &state,
            &session.id,
            "assistant",
            "ran a tool",
            Some("json"),
            Some(&payload),
        )
        .expect("append structured message");

        let reloaded = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("reload");
        let msg = &reloaded[0].messages[0];
        assert_eq!(msg["text"], "ran a tool");
        assert_eq!(msg["toolCalls"][0]["toolCallId"], "t1");
    }

    #[test]
    fn save_app_message_updates_existing_row_by_client_id_without_duplicates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");

        // First checkpoint during stream
        save_app_message_internal(
            &state,
            &session.id,
            Some("stream-123"),
            "Developer",
            "working...",
            Some("json"),
            Some(r#"{"text":"working...","toolCalls":[{"toolCallId":"t1","status":"running"}]}"#),
        )
        .expect("checkpoint 1");

        let first_load = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("load 1");
        assert_eq!(first_load[0].messages.len(), 1);
        assert_eq!(first_load[0].messages[0]["id"], "stream-123");
        assert_eq!(first_load[0].messages[0]["text"], "working...");

        // Second checkpoint (tool finished, more output)
        save_app_message_internal(
            &state,
            &session.id,
            Some("stream-123"),
            "Developer",
            "done with task!",
            Some("json"),
            Some(r#"{"text":"done with task!","toolCalls":[{"toolCallId":"t1","status":"completed"}]}"#),
        )
        .expect("checkpoint 2");

        let second_load = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("load 2");
        // Must NOT create a second message row — must update the existing one!
        assert_eq!(second_load[0].messages.len(), 1);
        assert_eq!(second_load[0].messages[0]["id"], "stream-123");
        assert_eq!(second_load[0].messages[0]["text"], "done with task!");
        assert_eq!(
            second_load[0].messages[0]["toolCalls"][0]["status"],
            "completed"
        );
    }

    #[test]
    fn imported_provider_session_ids_are_qualified_by_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let claude = upsert_imported_session(
            &state,
            None,
            "Claude",
            "/project",
            "claude-native",
            "shared-id",
            1,
            1,
        )
        .expect("import Claude");
        let codex = upsert_imported_session(
            &state,
            None,
            "Codex",
            "/project",
            "codex-cli",
            "shared-id",
            1,
            1,
        )
        .expect("import Codex");

        assert_ne!(claude.id, codex.id);
    }

    #[test]
    fn changing_session_runtime_isolates_and_preserves_overrides_per_engine() {
        use crate::db::app_session_role as roles;
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        let load = |runtime: &str| {
            roles::load_app_session_role_runtime_state(&state, &session.id, "Developer", runtime)
                .expect("load state")
        };

        roles::save_app_session_role_config_option_override(
            &state,
            &session.id,
            "Developer",
            "claude-native",
            "fast",
            "true",
        )
        .expect("save Claude option");
        roles::bind_app_session_role_runtime(&state, &session.id, "Developer", "codex-cli")
            .expect("switch runtime");
        assert_eq!(
            roles::load_app_session_bound_runtime(&state, &session.id, "Developer").expect("bound"),
            Some("codex-cli".to_string())
        );

        // Codex has no overrides from Claude (no leakage)…
        assert!(load("codex-cli")
            .and_then(|s| s.config_options_json)
            .is_none());
        // …and Claude's are preserved, not wiped, so switching back restores them.
        roles::bind_app_session_role_runtime(&state, &session.id, "Developer", "claude-native")
            .expect("switch back");
        assert!(load("claude-native")
            .and_then(|s| s.config_options_json)
            .unwrap_or_default()
            .contains("fast"));
    }

    #[test]
    fn provider_session_ids_are_kept_per_runtime_and_reset_is_scoped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        let ids = [
            ("claude-native", "claude-conversation-42"),
            ("antigravity-cli", "agy-conversation-17"),
            ("codex-cli", "codex-thread-9"),
            ("pi-cli", "pi-session-3"),
            ("claude-code", "acp-session-8"),
        ];

        for (runtime, provider_id) in ids {
            crate::db::app_session_role::save_app_session_role_cli_id(
                &state,
                &session.id,
                runtime,
                "Developer",
                provider_id,
            )
            .expect("save provider session id");
        }

        for (runtime, provider_id) in ids {
            assert_eq!(
                crate::db::app_session_role::load_app_session_role_cli_id(
                    &state,
                    &session.id,
                    runtime,
                    "Developer",
                )
                .as_deref(),
                Some(provider_id)
            );
        }

        crate::db::app_session_role::clear_app_session_role_cli_id(
            &state,
            &session.id,
            "Developer",
            "codex-cli",
        )
        .expect("clear Codex provider session id");

        assert_eq!(
            crate::db::app_session_role::load_app_session_role_cli_id(
                &state,
                &session.id,
                "codex-cli",
                "Developer",
            ),
            None
        );
        assert_eq!(
            crate::db::app_session_role::load_app_session_role_cli_id(
                &state,
                &session.id,
                "antigravity-cli",
                "Developer",
            )
            .as_deref(),
            Some("agy-conversation-17")
        );
    }

    #[test]
    fn corrupt_payload_falls_back_to_plain_text_instead_of_failing_the_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");

        append_app_message_internal(
            &state,
            &session.id,
            "assistant",
            "fallback text",
            Some("json"),
            Some("not valid json"),
        )
        .expect("append with corrupt payload");

        let reloaded = get_app_sessions_by_ids(&state, &[session.id.clone()])
            .expect("reload must not error on a corrupt payload");
        assert_eq!(reloaded[0].messages[0]["text"], "fallback text");
    }

    #[test]
    fn one_query_groups_messages_onto_the_right_sessions_in_order() {
        // Loading many sessions at once used to issue a query per session. Batching them
        // means the grouping is now this function's job, so it has to be pinned down.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let a = create_app_session_internal(&state, Some("a"), None, None, None, None)
            .expect("create a");
        let b = create_app_session_internal(&state, Some("b"), None, None, None, None)
            .expect("create b");

        // Interleaved so a naive grouping cannot pass by accident.
        append_app_message_internal(&state, &a.id, "user", "a1", None, None).expect("a1");
        append_app_message_internal(&state, &b.id, "user", "b1", None, None).expect("b1");
        append_app_message_internal(&state, &a.id, "user", "a2", None, None).expect("a2");
        append_app_message_internal(&state, &b.id, "user", "b2", None, None).expect("b2");

        let reloaded =
            get_app_sessions_by_ids(&state, &[a.id.clone(), b.id.clone()]).expect("reload");
        let texts = |id: &str| -> Vec<String> {
            reloaded
                .iter()
                .find(|s| s.id == id)
                .expect("session")
                .messages
                .iter()
                .map(|m| m["text"].as_str().unwrap_or_default().to_string())
                .collect()
        };
        assert_eq!(texts(&a.id), vec!["a1", "a2"]);
        assert_eq!(texts(&b.id), vec!["b1", "b2"]);
    }

    #[test]
    fn message_ids_are_stable_across_reloads() {
        // A fresh uuid per load made the id useless as a cache key: every reload was a total
        // miss for the rendered-markdown cache, re-parsing the whole visible history.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");
        append_app_message_internal(&state, &session.id, "user", "hi", None, None).expect("append");

        let first = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("load");
        let second = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("reload");
        assert_eq!(first[0].messages[0]["id"], second[0].messages[0]["id"]);
        assert!(!first[0].messages[0]["id"].as_str().unwrap().is_empty());
    }

    #[test]
    fn plain_text_message_keeps_working_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");

        append_app_message_internal(&state, &session.id, "user", "hi", None, None)
            .expect("append plain message");

        let reloaded = get_app_sessions_by_ids(&state, &[session.id.clone()]).expect("reload");
        let msg = &reloaded[0].messages[0];
        assert_eq!(msg["text"], "hi");
        assert!(msg.get("toolCalls").is_none());
    }

    #[test]
    fn fork_copies_transcript_and_dedupes_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let source =
            create_app_session_internal(&state, Some("s"), None, Some("codex-cli"), None, None)
                .expect("create session");
        save_app_message_internal(&state, &source.id, Some("c1"), "user", "hi", None, None)
            .unwrap();
        append_app_message_internal(&state, &source.id, "Developer", "yo", None, None).unwrap();

        let a = fork_app_session_internal(&state, &source.id, "[Fork]_s", None).unwrap();
        let b = fork_app_session_internal(&state, &source.id, "[Fork]_s", None).unwrap();
        assert_eq!(
            (a.title.as_str(), b.title.as_str()),
            ("[Fork]_s", "[Fork]_s_2")
        );
        assert_eq!(a.runtime_kind.as_deref(), Some("codex-cli"));
        assert_ne!(a.id, source.id);

        let copy = get_app_sessions_by_ids(&state, &[a.id.clone()]).unwrap();
        assert_eq!(copy[0].messages.len(), 2);
        assert_eq!(copy[0].messages[0]["text"], "hi");
        let original = get_app_sessions_by_ids(&state, &[source.id.clone()]).unwrap();
        assert_eq!(original[0].messages.len(), 2);
        assert!(fork_app_session_internal(&state, "missing", "x", None).is_err());
    }

    #[test]
    fn fork_binds_handle_and_copies_overrides() {
        use crate::db::app_session_role as roles;
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let source =
            create_app_session_internal(&state, Some("s"), None, Some("codex-cli"), None, None)
                .unwrap();
        roles::save_app_session_role_cli_id(&state, &source.id, "codex-cli", "Developer", "src")
            .unwrap();
        roles::save_app_session_role_mode_override(
            &state,
            &source.id,
            "Developer",
            "codex-cli",
            Some("plan"),
        )
        .unwrap();

        let fork = fork_app_session_internal(
            &state,
            &source.id,
            "[Fork]_s",
            Some(("codex-cli", "Developer", "forked")),
        )
        .unwrap();
        assert_eq!(
            roles::load_app_session_role_cli_id(&state, &fork.id, "codex-cli", "Developer")
                .as_deref(),
            Some("forked")
        );
        let copied =
            roles::load_app_session_role_runtime_state(&state, &fork.id, "Developer", "codex-cli")
                .unwrap()
                .unwrap();
        assert_eq!(copied.mode_override.as_deref(), Some("plan"));
        assert_eq!(
            roles::load_app_session_role_cli_id(&state, &source.id, "codex-cli", "Developer")
                .as_deref(),
            Some("src")
        );
    }

    #[test]
    fn fork_leaves_no_session_when_the_handle_cannot_be_saved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let source =
            create_app_session_internal(&state, Some("s"), None, Some("codex-cli"), None, None)
                .unwrap();
        with_db(&state, |conn| {
            conn.execute("DROP TABLE app_session_role_runtime_configs", [])
                .map_err(|e| e.to_string())
        })
        .unwrap();

        let err = fork_app_session_internal(
            &state,
            &source.id,
            "[Fork]_s",
            Some(("codex-cli", "Developer", "forked")),
        );
        assert!(err.is_err());
        let count: i64 = with_db(&state, |conn| {
            conn.query_row("SELECT count(*) FROM app_sessions", [], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(count, 1);
        assert!(fork_app_session_internal(
            &state,
            &source.id,
            "x",
            Some(("codex-cli", "Developer", " "))
        )
        .is_err());
    }
}
