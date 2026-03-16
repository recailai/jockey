pub(crate) mod context;
pub(crate) mod role;
pub(crate) mod session;
pub(crate) mod workflow;

use crate::types::*;
use crate::now_ms;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use tauri::State;
use uuid::Uuid;

pub(crate) fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS teams (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          workspace_path TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS roles (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          system_prompt TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          UNIQUE(team_id, role_name)
        );
        CREATE TABLE IF NOT EXISTS workflows (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          name TEXT NOT NULL,
          steps_json TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          workflow_id TEXT NOT NULL,
          status TEXT NOT NULL,
          initial_prompt TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          event_type TEXT NOT NULL,
          role_name TEXT,
          payload TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS shared_context_snapshots (
          team_id TEXT NOT NULL,
          key TEXT NOT NULL,
          value TEXT NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(team_id, key)
        );
        CREATE TABLE IF NOT EXISTS dynamic_catalog_entries (
          kind TEXT NOT NULL,
          name TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(kind, name)
        );
        CREATE INDEX IF NOT EXISTS idx_roles_team_updated_at
          ON roles(team_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_workflows_team_updated_at
          ON workflows(team_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_team_created_at
          ON sessions(team_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_workflow_id
          ON sessions(workflow_id);
        CREATE INDEX IF NOT EXISTS idx_session_events_session_id_id
          ON session_events(session_id, id ASC);
        CREATE INDEX IF NOT EXISTS idx_shared_context_team_updated_at
          ON shared_context_snapshots(team_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_dynamic_catalog_kind_updated_at
          ON dynamic_catalog_entries(kind, updated_at DESC);
        ",
    )
    .map_err(|e| e.to_string())?;

    let alter_stmts = [
        "ALTER TABLE roles ADD COLUMN model TEXT DEFAULT NULL",
        "ALTER TABLE roles ADD COLUMN mode TEXT DEFAULT NULL",
        "ALTER TABLE roles ADD COLUMN mcp_servers_json TEXT DEFAULT '[]'",
        "ALTER TABLE roles ADD COLUMN config_options_json TEXT DEFAULT '{}'",
        "ALTER TABLE roles ADD COLUMN auto_approve INTEGER DEFAULT 1",
    ];
    for stmt in alter_stmts {
        let _ = conn.execute(stmt, []);
    }

    Ok(())
}

pub(crate) fn get_state<'a>(state: &'a State<'_, AppState>) -> &'a AppState {
    state.inner()
}

pub(crate) fn with_db<T>(
    state: &AppState,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    f(&conn)
}

pub(crate) fn parse_payload(payload: String) -> Value {
    serde_json::from_str::<Value>(&payload).unwrap_or(json!({ "text": payload }))
}

pub(crate) fn seed_default_dynamic_catalog(state: &AppState) -> Result<(), String> {
    for model in DEFAULT_MODELS {
        let _ = context::upsert_dynamic_catalog_item(state, "model", model)?;
    }
    for server in DEFAULT_MCP_SERVERS {
        let _ = context::upsert_dynamic_catalog_item(state, "mcp", server)?;
    }
    for skill in DEFAULT_SKILLS {
        let _ = context::upsert_dynamic_catalog_item(state, "skill", skill)?;
    }
    Ok(())
}

pub(crate) fn team_exists(state: &AppState, team_id: &str) -> Result<bool, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT 1 FROM teams WHERE id = ?1 LIMIT 1",
            params![team_id],
            |_row| Ok(()),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|v| v.is_some())
    })
}

pub(crate) fn ensure_default_team_id(state: &AppState) -> Result<String, String> {
    let existing = with_db(state, |conn| {
        conn.query_row(
            "SELECT id FROM teams ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let team = Team {
        id: Uuid::new_v4().to_string(),
        name: DEFAULT_WORKSPACE_NAME.to_string(),
        workspace_path: ".".to_string(),
        created_at: now_ms(),
    };
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO teams (id, name, workspace_path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![&team.id, &team.name, &team.workspace_path, team.created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(team.id)
}

pub(crate) fn load_team_workspace_path(state: &AppState, team_id: &str) -> Result<String, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT workspace_path FROM teams WHERE id = ?1",
            params![team_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
}
