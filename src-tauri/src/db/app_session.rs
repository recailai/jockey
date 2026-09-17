use crate::acp;
use crate::db::app_session_role::clear_app_session_role_cli_id;
use crate::db::{get_state, with_db};
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
        "SELECT session_id, id, role_name, content, content_type, payload, created_at
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
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut by_session: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for row in rows {
        let (session_id, message_id, role_name, content, content_type, payload, at) =
            row.map_err(|e| e.to_string())?;
        let mut obj = serde_json::json!({
            // The table's own autoincrement key. Minting a fresh uuid per load made every
            // reload a total miss for anything keyed on message id, notably the rendered
            // markdown cache, which then re-parsed the whole visible history.
            "id": format!("m{message_id}"),
            "roleName": role_name,
            "text": content,
            "at": at
        });
        // Structured extras (toolCalls/segments/images/thoughtText) merge over the plain
        // base — a corrupt payload just falls back to the plain-text object rather than
        // failing the whole session load, same defensive stance as `parse_payload`.
        if content_type == "json" {
            if let Some(raw) = payload {
                if let Ok(serde_json::Value::Object(extra)) =
                    serde_json::from_str::<serde_json::Value>(&raw)
                {
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

#[tauri::command]
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

#[tauri::command]
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

pub(crate) fn append_app_message_internal(
    state: &AppState,
    session_id: &str,
    role_name: &str,
    content: &str,
    content_type: Option<&str>,
    payload: Option<&str>,
) -> Result<(), String> {
    let now = now_ms();
    let content_type = content_type
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("text");
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO app_session_messages (session_id, role_name, content, content_type, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, role_name, content, content_type, payload, now],
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

pub(crate) fn insert_imported_messages(
    state: &AppState,
    session_id: &str,
    messages: &[(String, String, i64)],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "INSERT INTO app_session_messages (session_id, role_name, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| e.to_string())?;
        for (role_name, content, created_at) in messages {
            stmt.execute(params![session_id, role_name, content, created_at])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
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
        let mut final_title = base_title.clone();
        let mut counter = 1;
        while active_session_title_exists(conn, &final_title, project_id, None)? {
            counter += 1;
            final_title = format!("{}_{}", base_title, counter);
        }

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
            conn.execute(
                "UPDATE app_sessions SET last_active_at = ?1, project_id = coalesce(project_id, ?2), cwd = ?3 WHERE id = ?4",
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
        let mut session_title = title.replace(' ', "_");
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
}

pub(crate) fn close_app_session_internal(state: &AppState, id: &str) -> Result<(), String> {
    let now = now_ms();
    with_db(state, |conn| {
        let changed = conn
            .execute(
                "UPDATE app_sessions SET closed_at = ?1 WHERE id = ?2 AND closed_at IS NULL",
                params![now, id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("session not found or already closed: {id}"));
        }
        Ok(())
    })?;
    schedule_session_role_cleanup(state.clone_refs(), id.to_string(), false);
    Ok(())
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
            let _ = acp::reset_session(&runtime_kind, &role_name, Some(&session_id)).await;
            let _ = clear_app_session_role_cli_id(&state, &session_id, &role_name);
        }
    });
}

#[tauri::command]
pub(crate) fn create_app_session(
    state: State<'_, AppState>,
    title: Option<String>,
    project_id: Option<String>,
    runtime_kind: Option<String>,
    runtime_profile_id: Option<String>,
    id: Option<String>,
) -> Result<AppSession, String> {
    create_app_session_internal(
        get_state(&state),
        title.as_deref(),
        project_id.as_deref(),
        runtime_kind.as_deref(),
        runtime_profile_id.as_deref(),
        id.as_deref(),
    )
}

#[tauri::command]
pub(crate) async fn update_app_session(
    state: State<'_, AppState>,
    id: String,
    update: AppSessionUpdate,
) -> Result<(), String> {
    let role_changed = update.active_role.is_some();
    with_db(get_state(&state), |conn| {
        let current_project_id: Option<String> = conn
            .query_row(
                "SELECT project_id FROM app_sessions WHERE id = ?1",
                params![&id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();

        if let Some(ref title) = update.title {
            let title = validate_session_title(title)?;
            if active_session_title_exists(conn, &title, current_project_id.as_deref(), Some(&id))?
            {
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
    })?;
    if role_changed {
        schedule_session_role_cleanup(get_state(&state).clone_refs(), id, true);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn delete_app_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
        conn.execute(
            "UPDATE app_sessions SET closed_at = ?1 WHERE id = ?2",
            params![now, &id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    schedule_session_role_cleanup(get_state(&state).clone_refs(), id, false);
    Ok(())
}

#[tauri::command]
pub(crate) fn reopen_app_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<AppSession, String> {
    let now = now_ms();
    with_db(get_state(&state), |conn| {
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
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let session = create_app_session_internal(&state, Some("s"), None, None, None, None)
            .expect("create session");

        crate::db::app_session_role::save_app_session_role_model_override(
            &state,
            &session.id,
            "Developer",
            "claude-native",
            Some("claude-sonnet"),
        )
        .expect("save Claude model");
        crate::db::app_session_role::save_app_session_role_config_option_override(
            &state,
            &session.id,
            "Developer",
            "claude-native",
            "fast",
            "true",
        )
        .expect("save Claude option");

        crate::db::app_session_role::bind_app_session_role_runtime(
            &state,
            &session.id,
            "Developer",
            "codex-cli",
        )
        .expect("switch runtime");
        let codex_state = crate::db::app_session_role::load_app_session_role_state(
            &state,
            &session.id,
            "Developer",
        )
        .expect("load state")
        .expect("state");

        // Codex has no overrides from Claude (no leakage)
        assert_eq!(codex_state.runtime_kind.as_deref(), Some("codex-cli"));
        assert!(codex_state.model_override.is_none());
        assert!(codex_state.config_options_json.is_none());

        // Claude overrides are preserved, not wiped!
        let claude_state = crate::db::app_session_role::load_app_session_role_runtime_state(
            &state,
            &session.id,
            "Developer",
            "claude-native",
        )
        .expect("load claude state")
        .expect("claude state");

        assert_eq!(
            claude_state.model_override.as_deref(),
            Some("claude-sonnet")
        );
        assert!(claude_state
            .config_options_json
            .as_deref()
            .unwrap_or("")
            .contains("fast"));

        // Switching back to Claude restores them seamlessly
        crate::db::app_session_role::bind_app_session_role_runtime(
            &state,
            &session.id,
            "Developer",
            "claude-native",
        )
        .expect("switch back");
        let restored_state = crate::db::app_session_role::load_app_session_role_state(
            &state,
            &session.id,
            "Developer",
        )
        .expect("load state")
        .expect("state");
        assert_eq!(
            restored_state.model_override.as_deref(),
            Some("claude-sonnet")
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
}
