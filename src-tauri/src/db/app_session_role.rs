use crate::db::with_db;
use crate::now_ms;
use crate::runtime_profile;
use crate::types::AppState;
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

#[derive(Clone, Default, Debug)]
pub(crate) struct AppSessionRoleState {
    // Runtime resolution reads the binding through `load_app_session_bound_runtime`.
    #[allow(dead_code)]
    pub(crate) runtime_kind: Option<String>,
    #[allow(dead_code)]
    pub(crate) acp_session_id: Option<String>,
    pub(crate) model_override: Option<String>,
    pub(crate) mode_override: Option<String>,
    pub(crate) config_options_json: Option<String>,
}

fn ensure_app_session_role_row(
    conn: &rusqlite::Connection,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
) -> Result<(), String> {
    let runtime_profile_id = runtime_profile::profile_id(runtime_kind);
    conn.execute(
        "INSERT INTO app_session_roles (
            app_session_id, role_name, runtime_kind, runtime_profile_id
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(app_session_id, role_name) DO UPDATE SET
           runtime_kind = excluded.runtime_kind,
           runtime_profile_id = excluded.runtime_profile_id",
        params![app_session_id, role_name, runtime_kind, runtime_profile_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_app_session_role_runtime_config_row(
    conn: &rusqlite::Connection,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
) -> Result<(), String> {
    let now = now_ms();
    let runtime_profile_id = runtime_profile::profile_id(runtime_kind);
    conn.execute(
        "INSERT INTO app_session_role_runtime_configs (
            app_session_id, role_name, runtime_kind, runtime_profile_id, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
           runtime_profile_id = excluded.runtime_profile_id,
           updated_at = excluded.updated_at",
        params![
            app_session_id,
            role_name,
            runtime_kind,
            runtime_profile_id,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Rebind this session's role to a different engine.
///
/// Keeps each engine's session-level overrides (model, mode, options, acp_session_id)
/// segregated in `app_session_role_runtime_configs` so switching back preserves the previous engine's setup.
pub(crate) fn bind_app_session_role_runtime(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    with_db(state, |conn| {
        ensure_app_session_role_row(conn, app_session_id, role_name, runtime_kind)?;
        ensure_app_session_role_runtime_config_row(conn, app_session_id, role_name, runtime_kind)?;
        Ok(())
    })
}

pub(crate) fn load_app_session_bound_runtime(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Result<Option<String>, String> {
    if app_session_id.trim().is_empty() {
        return Ok(None);
    }
    with_db(state, |conn| {
        conn.query_row(
            "SELECT runtime_kind FROM app_session_roles
             WHERE app_session_id = ?1 AND role_name = ?2",
            params![app_session_id, role_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn load_app_session_role_runtime_state(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
) -> Result<Option<AppSessionRoleState>, String> {
    if app_session_id.trim().is_empty() {
        return Ok(None);
    }
    with_db(state, |conn| {
        let hit: Option<AppSessionRoleState> = conn
            .query_row(
                "SELECT runtime_kind, acp_session_id, model_override, mode_override, config_options_json
                 FROM app_session_role_runtime_configs
                 WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
                params![app_session_id, role_name, runtime_kind],
                |row| {
                    let cfg_json = row
                        .get::<_, Option<String>>(4)?
                        .filter(|s| !s.trim().is_empty() && s.trim() != "{}");
                    Ok(AppSessionRoleState {
                        runtime_kind: row.get(0)?,
                        acp_session_id: row.get(1)?,
                        model_override: row.get(2)?,
                        mode_override: row.get(3)?,
                        config_options_json: cfg_json,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some(state) = hit {
            return Ok(Some(state));
        }

        // Fallback for un-migrated or legacy rows in app_session_roles
        conn.query_row(
            "SELECT runtime_kind, acp_session_id, model_override, mode_override, config_options_json
             FROM app_session_roles
             WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
            params![app_session_id, role_name, runtime_kind],
            |row| {
                let cfg_json = row
                    .get::<_, Option<String>>(4)?
                    .filter(|s| !s.trim().is_empty() && s.trim() != "{}");
                Ok(AppSessionRoleState {
                    runtime_kind: row.get(0)?,
                    acp_session_id: row.get(1)?,
                    model_override: row.get(2)?,
                    mode_override: row.get(3)?,
                    config_options_json: cfg_json,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn save_app_session_role_mode_override(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
    mode_override: Option<&str>,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let now = now_ms();
    let profile_id = runtime_profile::profile_id(runtime_kind);
    with_db(state, |conn| {
        ensure_app_session_role_row(conn, app_session_id, role_name, runtime_kind)?;
        conn.execute(
            "INSERT INTO app_session_role_runtime_configs (
                app_session_id, role_name, runtime_kind, runtime_profile_id, mode_override, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
               mode_override = excluded.mode_override,
               runtime_profile_id = excluded.runtime_profile_id,
               updated_at = excluded.updated_at",
            params![
                app_session_id,
                role_name,
                runtime_kind,
                &profile_id,
                mode_override,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE app_session_roles
             SET mode_override = ?1, runtime_kind = ?2, runtime_profile_id = ?3
             WHERE app_session_id = ?4 AND role_name = ?5",
            params![
                mode_override,
                runtime_kind,
                &profile_id,
                app_session_id,
                role_name
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn save_app_session_role_config_option_override(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
    config_id: &str,
    value: &str,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    if config_id.trim().is_empty() {
        return Err("config id required".to_string());
    }
    let now = now_ms();
    let profile_id = runtime_profile::profile_id(runtime_kind);
    with_db(state, |conn| {
        ensure_app_session_role_row(conn, app_session_id, role_name, runtime_kind)?;
        let current_json: Option<String> = conn
            .query_row(
                "SELECT config_options_json
                 FROM app_session_role_runtime_configs
                 WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
                params![app_session_id, role_name, runtime_kind],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten()
            .or_else(|| {
                // Fallback to app_session_roles
                conn.query_row(
                    "SELECT config_options_json
                     FROM app_session_roles
                     WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
                    params![app_session_id, role_name, runtime_kind],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten()
            });

        let mut map: Map<String, Value> = current_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();

        if value.trim().is_empty() {
            map.remove(config_id);
        } else {
            map.insert(config_id.to_string(), Value::String(value.to_string()));
        }

        let next_json = serde_json::to_string(&map).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO app_session_role_runtime_configs (
                app_session_id, role_name, runtime_kind, runtime_profile_id, config_options_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
               config_options_json = excluded.config_options_json,
               runtime_profile_id = excluded.runtime_profile_id,
               updated_at = excluded.updated_at",
            params![
                app_session_id,
                role_name,
                runtime_kind,
                &profile_id,
                &next_json,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE app_session_roles
             SET config_options_json = ?1, runtime_kind = ?2, runtime_profile_id = ?3
             WHERE app_session_id = ?4 AND role_name = ?5",
            params![
                &next_json,
                runtime_kind,
                &profile_id,
                app_session_id,
                role_name
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
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
        let hit: Option<String> = conn
            .query_row(
                "SELECT acp_session_id FROM app_session_role_runtime_configs
                 WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
                params![app_session_id, role_name, runtime_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();

        if hit.is_some() {
            return Ok(hit);
        }

        // Fallback to app_session_roles
        conn.query_row(
            "SELECT acp_session_id FROM app_session_roles
             WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
            params![app_session_id, role_name, runtime_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|v| v.flatten())
    })
    .ok()
    .flatten()
}

pub(crate) fn save_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
    cli_session_id: &str,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() || cli_session_id.trim().is_empty() {
        return Ok(());
    }
    with_db(state, |conn| {
        save_cli_id_on(conn, app_session_id, runtime_key, role_name, cli_session_id)
    })
}

/// Connection-level body of [`save_app_session_role_cli_id`], so a caller can run it inside
/// its own transaction.
pub(crate) fn save_cli_id_on(
    conn: &rusqlite::Connection,
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
    cli_session_id: &str,
) -> Result<(), String> {
    let now = now_ms();
    let profile_id = runtime_profile::profile_id(runtime_key);
    ensure_app_session_role_row(conn, app_session_id, role_name, runtime_key)?;
    conn.execute(
        "INSERT INTO app_session_role_runtime_configs (
            app_session_id, role_name, runtime_kind, runtime_profile_id, acp_session_id, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
           -- A different provider conversation starts from its own beginning.
           transcript_cursor = CASE WHEN acp_session_id IS excluded.acp_session_id
                                    THEN transcript_cursor ELSE NULL END,
           transcript_offset = CASE WHEN acp_session_id IS excluded.acp_session_id
                                    THEN transcript_offset ELSE NULL END,
           transcript_fingerprint = CASE WHEN acp_session_id IS excluded.acp_session_id
                                    THEN transcript_fingerprint ELSE NULL END,
           acp_session_id = excluded.acp_session_id,
           runtime_profile_id = excluded.runtime_profile_id,
           updated_at = excluded.updated_at",
        params![
            app_session_id,
            role_name,
            runtime_key,
            &profile_id,
            cli_session_id,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE app_session_roles
         SET runtime_kind = ?1, runtime_profile_id = ?2, acp_session_id = ?3
         WHERE app_session_id = ?4 AND role_name = ?5",
        params![
            runtime_key,
            &profile_id,
            cli_session_id,
            app_session_id,
            role_name
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Copies the session-level model, mode and option overrides of one (persona, engine) binding
/// to another session, so a fork keeps the setup its source was running with.
pub(crate) fn copy_role_overrides_on(
    conn: &rusqlite::Connection,
    from_session_id: &str,
    to_session_id: &str,
    role_name: &str,
    runtime_key: &str,
) -> Result<(), String> {
    let copied = conn
        .execute(
            "UPDATE app_session_role_runtime_configs SET
               model_override = src.model_override,
               mode_override = src.mode_override,
               config_options_json = src.config_options_json
             FROM (SELECT model_override, mode_override, config_options_json
                   FROM app_session_role_runtime_configs
                   WHERE app_session_id = ?1 AND role_name = ?3 AND runtime_kind = ?4) AS src
             WHERE app_session_id = ?2 AND role_name = ?3 AND runtime_kind = ?4",
            params![from_session_id, to_session_id, role_name, runtime_key],
        )
        .map_err(|e| e.to_string())?;
    if copied == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE app_session_roles SET
           model_override = src.model_override,
           mode_override = src.mode_override,
           config_options_json = src.config_options_json
         FROM (SELECT model_override, mode_override, config_options_json
               FROM app_session_role_runtime_configs
               WHERE app_session_id = ?2 AND role_name = ?3 AND runtime_kind = ?4) AS src
         WHERE app_session_id = ?2 AND role_name = ?3",
        params![from_session_id, to_session_id, role_name, runtime_key],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// A provider conversation behind an app session: one per (persona, engine) that has a
/// provider handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ProviderConversation {
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) provider_id: String,
    /// Where reading the provider transcript resumes: the byte offset after the last
    /// fully-imported line plus a fingerprint of the file around it. `None` until the first
    /// sync (or for a binding that predates byte positions).
    pub(crate) position: Option<TranscriptPosition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TranscriptPosition {
    pub(crate) offset: u64,
    pub(crate) fingerprint: String,
}

fn query_conversations(
    conn: &rusqlite::Connection,
    app_session_id: Option<&str>,
) -> Result<Vec<(String, ProviderConversation)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT app_session_id, role_name, runtime_kind, acp_session_id,
                    transcript_offset, transcript_fingerprint
             FROM app_session_role_runtime_configs
             WHERE (?1 IS NULL OR app_session_id = ?1)
               AND acp_session_id IS NOT NULL AND acp_session_id != ''",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![app_session_id], |row| {
            let offset = row.get::<_, Option<i64>>(4)?;
            let fingerprint = row.get::<_, Option<String>>(5)?;
            Ok((
                row.get::<_, String>(0)?,
                ProviderConversation {
                    role_name: row.get(1)?,
                    runtime_kind: row.get(2)?,
                    provider_id: row.get(3)?,
                    position: offset.zip(fingerprint).map(|(o, f)| TranscriptPosition {
                        offset: o.max(0) as u64,
                        fingerprint: f,
                    }),
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    // A legacy alias row (e.g. `claude-native`) can still hold the handle its canonical
    // successor took over; following both imports the same transcript twice.
    let is_canonical = |c: &ProviderConversation| {
        crate::runtime_kind::RuntimeKind::from_str(&c.runtime_kind)
            .is_some_and(|k| k.runtime_key() == c.runtime_kind)
    };
    rows.sort_by_key(|(_, c)| !is_canonical(c));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|(sid, c)| seen.insert((sid.clone(), c.role_name.clone(), c.provider_id.clone())));
    Ok(rows)
}

/// Gives a session that was imported (it has an `external_session_id`) but lost its provider
/// handle the binding back. Closing a session used to erase the handle; the imported id is the
/// authoritative record of which provider conversation the session follows. A no-op when the
/// session already has a handle for its engine. Returns whether a binding was written.
pub(crate) fn rebind_from_external(
    conn: &rusqlite::Connection,
    app_session_id: &str,
) -> Result<bool, String> {
    let row: Option<(Option<String>, Option<String>, String)> = conn
        .query_row(
            "SELECT external_session_id, runtime_kind, active_role FROM app_sessions WHERE id = ?1",
            params![app_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((Some(external), Some(runtime), role)) = row else {
        return Ok(false);
    };
    if external.trim().is_empty() || runtime.trim().is_empty() {
        return Ok(false);
    }
    let canonical = crate::runtime_kind::RuntimeKind::from_str(&runtime)
        .map_or(runtime.clone(), |k| k.runtime_key().to_string());
    let bound: i64 = conn
        .query_row(
            "SELECT count(1) FROM app_session_role_runtime_configs
             WHERE app_session_id = ?1 AND role_name = ?2
               AND acp_session_id IS NOT NULL AND trim(acp_session_id) <> ''",
            params![app_session_id, role],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if bound > 0 {
        return Ok(false);
    }
    save_cli_id_on(conn, app_session_id, &canonical, &role, &external)?;
    Ok(true)
}

pub(crate) fn list_provider_conversations(
    state: &AppState,
    app_session_id: &str,
) -> Result<Vec<ProviderConversation>, String> {
    with_db(state, |conn| {
        let mut rows = query_conversations(conn, Some(app_session_id))?;
        if rows.is_empty() && rebind_from_external(conn, app_session_id)? {
            rows = query_conversations(conn, Some(app_session_id))?;
        }
        Ok(rows.into_iter().map(|(_, c)| c).collect())
    })
}

/// Every provider conversation of every session, open or closed (the background sync's work
/// list).
pub(crate) fn list_all_provider_conversations(
    state: &AppState,
) -> Result<Vec<(String, ProviderConversation)>, String> {
    with_db(state, |conn| {
        let unbound: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT s.id FROM app_sessions s
                     WHERE s.external_session_id IS NOT NULL AND trim(s.external_session_id) <> ''
                       AND NOT EXISTS (
                         SELECT 1 FROM app_session_role_runtime_configs c
                         WHERE c.app_session_id = s.id
                           AND c.acp_session_id IS NOT NULL AND trim(c.acp_session_id) <> '')",
                )
                .map_err(|e| e.to_string())?;
            let ids = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();
            ids
        };
        for id in unbound {
            rebind_from_external(conn, &id)?;
        }
        query_conversations(conn, None)
    })
}

/// Only advances the position of the conversation it was read for: if the persona has moved to
/// a different provider conversation since, the stale write is dropped.
pub(crate) fn set_transcript_position(
    state: &AppState,
    app_session_id: &str,
    conversation: &ProviderConversation,
    position: &TranscriptPosition,
) -> Result<(), String> {
    with_db(state, |conn| {
        conn.execute(
            "UPDATE app_session_role_runtime_configs
             SET transcript_offset = ?1, transcript_fingerprint = ?2
             WHERE app_session_id = ?3 AND role_name = ?4 AND runtime_kind = ?5
               AND acp_session_id = ?6",
            params![
                position.offset as i64,
                position.fingerprint,
                app_session_id,
                conversation.role_name,
                conversation.runtime_kind,
                conversation.provider_id
            ],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn clear_app_session_role_cli_id(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_key: &str,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() || runtime_key.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    with_db(state, |conn| {
        // An explicit reset detaches the conversation for good, including the imported id the
        // binding would otherwise be restored from (`rebind_from_external`).
        conn.execute(
            "UPDATE app_sessions SET external_session_id = NULL
             WHERE id = ?1 AND external_session_id IN (
               SELECT acp_session_id FROM app_session_role_runtime_configs
               WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3)",
            params![app_session_id, role_name, runtime_key],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE app_session_role_runtime_configs
             SET acp_session_id = NULL, transcript_offset = NULL, transcript_fingerprint = NULL
             WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
            params![app_session_id, role_name, runtime_key],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE app_session_roles
             SET acp_session_id = NULL
             WHERE app_session_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
            params![app_session_id, role_name, runtime_key],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use crate::db::app_session::{
        close_app_session_internal, reopen_app_session_internal, upsert_imported_session,
    };
    use crate::db::{init_db, DbPool};

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(dir.path().join("t.sqlite3"), 2, "PRAGMA foreign_keys = ON;")
            .expect("pool");
        init_db(&pool.get().expect("conn")).expect("init");
        AppState {
            db: pool,
            role_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }

    fn imported(state: &AppState) -> String {
        let session =
            upsert_imported_session(state, None, "imported", "/w", "claude-code", "ext-1", 1, 1)
                .unwrap();
        save_app_session_role_cli_id(state, &session.id, "claude-code", "Developer", "ext-1")
            .unwrap();
        session.id
    }

    fn handles(state: &AppState, sid: &str) -> Vec<String> {
        list_provider_conversations(state, sid)
            .unwrap()
            .into_iter()
            .map(|c| c.provider_id)
            .collect()
    }

    #[test]
    fn closing_keeps_the_binding_and_a_wiped_one_is_restored_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let sid = imported(&state);
        // Closing only marks the row; the slot cleanup it schedules never touches the handle.
        close_app_session_internal(&state, &sid).unwrap();
        assert_eq!(handles(&state, &sid), ["ext-1"]);

        // What the old close did: the handle is gone, the imported id is not.
        with_db(&state, |conn| {
            conn.execute(
                "UPDATE app_session_role_runtime_configs SET acp_session_id = ''",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute("UPDATE app_session_roles SET acp_session_id = NULL", [])
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(
            handles(&state, &sid),
            ["ext-1"],
            "listing falls back to the imported id"
        );

        with_db(&state, |conn| {
            conn.execute(
                "UPDATE app_session_role_runtime_configs SET acp_session_id = NULL",
                [],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        reopen_app_session_internal(&state, &sid).unwrap();
        assert_eq!(
            load_app_session_role_cli_id(&state, &sid, "claude-code", "Developer").as_deref(),
            Some("ext-1"),
            "reopen rebinds"
        );
    }

    #[test]
    fn an_explicit_reset_detaches_the_conversation_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let sid = imported(&state);
        clear_app_session_role_cli_id(&state, &sid, "Developer", "claude-code").unwrap();
        assert!(handles(&state, &sid).is_empty());
    }

    #[test]
    fn the_repair_migration_restores_bindings_of_closed_imports() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let sid = imported(&state);
        with_db(&state, |conn| {
            conn.execute(
                "UPDATE app_session_role_runtime_configs SET acp_session_id = ''",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.pragma_update(None, "user_version", 18)
                .map_err(|e| e.to_string())
        })
        .unwrap();
        // The unbound, closed session is repaired by the one-time migration at startup.
        with_db(&state, init_db).unwrap();
        let bound: i64 = with_db(&state, |conn| {
            conn.query_row(
                "SELECT count(1) FROM app_session_role_runtime_configs
                 WHERE app_session_id = ?1 AND acp_session_id = 'ext-1'",
                [&sid],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(bound, 1);
        let version: i64 = with_db(&state, |conn| {
            conn.query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(version, 19);
    }

    #[test]
    fn a_new_provider_handle_resets_the_transcript_position() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let sid = imported(&state);
        let conversation = list_provider_conversations(&state, &sid)
            .unwrap()
            .pop()
            .unwrap();
        set_transcript_position(
            &state,
            &sid,
            &conversation,
            &TranscriptPosition {
                offset: 10,
                fingerprint: "f".into(),
            },
        )
        .unwrap();
        assert_eq!(
            list_provider_conversations(&state, &sid).unwrap()[0]
                .position
                .as_ref()
                .map(|p| p.offset),
            Some(10)
        );
        save_app_session_role_cli_id(&state, &sid, "claude-code", "Developer", "other").unwrap();
        assert!(list_provider_conversations(&state, &sid).unwrap()[0]
            .position
            .is_none());
    }
}
