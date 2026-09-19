use crate::db::with_db;
use crate::now_ms;
use crate::runtime_profile;
use crate::types::AppState;
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

#[derive(Clone, Default, Debug)]
pub(crate) struct AppSessionRoleState {
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

pub(crate) fn load_app_session_role_state(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Result<Option<AppSessionRoleState>, String> {
    if app_session_id.trim().is_empty() {
        return Ok(None);
    }
    let bound = load_app_session_bound_runtime(state, app_session_id, role_name)?;
    let Some(runtime) = bound else {
        return Ok(None);
    };
    load_app_session_role_runtime_state(state, app_session_id, role_name, &runtime)
}

pub(crate) fn save_app_session_role_model_override(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    runtime_kind: &str,
    model_override: Option<&str>,
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
                app_session_id, role_name, runtime_kind, runtime_profile_id, model_override, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
               model_override = excluded.model_override,
               runtime_profile_id = excluded.runtime_profile_id,
               updated_at = excluded.updated_at",
            params![
                app_session_id,
                role_name,
                runtime_kind,
                &profile_id,
                model_override,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;

        // Keep app_session_roles in sync for backward compatibility
        conn.execute(
            "UPDATE app_session_roles
             SET model_override = ?1, runtime_kind = ?2, runtime_profile_id = ?3
             WHERE app_session_id = ?4 AND role_name = ?5",
            params![
                model_override,
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
    let now = now_ms();
    let profile_id = runtime_profile::profile_id(runtime_key);
    with_db(state, |conn| {
        ensure_app_session_role_row(conn, app_session_id, role_name, runtime_key)?;
        conn.execute(
            "INSERT INTO app_session_role_runtime_configs (
                app_session_id, role_name, runtime_kind, runtime_profile_id, acp_session_id, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app_session_id, role_name, runtime_kind) DO UPDATE SET
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
        conn.execute(
            "UPDATE app_session_role_runtime_configs
             SET acp_session_id = NULL
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
