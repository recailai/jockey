pub(crate) mod app_session;
pub(crate) mod app_session_role;
pub(crate) mod context;
pub(crate) mod global_mcp;
pub(crate) mod inbox;
pub(crate) mod lifecycle;
pub(crate) mod pool;
pub(crate) mod profiles;
pub(crate) mod project;
pub(crate) mod project_agent;
pub(crate) mod role;
pub(crate) mod rule;
pub(crate) mod session;
pub(crate) mod session_context;
pub(crate) mod skill;
pub(crate) mod workflow;

pub(crate) use pool::DbPool;

use crate::types::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use tauri::State;

/// Bump when a new one-shot data migration is added below. Table and column creation stay
/// unconditional (they are idempotent); only data rewrites are gated on this.
const SCHEMA_VERSION: i64 = 16;

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    if !is_safe_identifier(table) || !is_safe_identifier(column) {
        return Err("invalid table or column name".to_string());
    }
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma).map_err(|e| e.to_string())?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    let found = columns.filter_map(Result::ok).any(|name| name == column);
    Ok(found)
}

fn is_safe_identifier(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> Result<(), String> {
    if !is_safe_identifier(table) {
        return Err(format!("invalid table name: {table}"));
    }
    if !is_safe_identifier(column) {
        return Err(format!("invalid column name: {column}"));
    }
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    let mut exists = false;
    for row in rows {
        let name = row.map_err(|e| e.to_string())?;
        if name == column {
            exists = true;
            break;
        }
    }
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {column_def}"),
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn init_db(conn: &Connection) -> Result<(), String> {
    let from_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS roles (
          id TEXT PRIMARY KEY,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          runtime_profile_id TEXT,
          system_prompt TEXT NOT NULL,
          model TEXT,
          mode TEXT,
          mcp_servers_json TEXT DEFAULT '[]',
          config_options_json TEXT DEFAULT '{}',
          config_option_defs_json TEXT DEFAULT '[]',
          auto_approve INTEGER DEFAULT 1,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workflows (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          steps_json TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY,
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
          scope TEXT NOT NULL,
          key TEXT NOT NULL,
          value TEXT NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(scope, key)
        );
        CREATE TABLE IF NOT EXISTS dynamic_catalog_entries (
          kind TEXT NOT NULL,
          runtime_key TEXT NOT NULL DEFAULT '',
          name TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(kind, runtime_key, name)
        );
        CREATE TABLE IF NOT EXISTS app_sessions (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          active_role TEXT NOT NULL DEFAULT 'Developer',
          runtime_kind TEXT,
          runtime_profile_id TEXT,
          cwd TEXT,
          created_at INTEGER NOT NULL,
          last_active_at INTEGER NOT NULL,
          closed_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS app_session_roles (
          app_session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          runtime_profile_id TEXT,
          acp_session_id TEXT,
          model_override TEXT,
          mode_override TEXT,
          mcp_servers_json TEXT,
          config_options_json TEXT,
          PRIMARY KEY(app_session_id, role_name)
        );
        CREATE TABLE IF NOT EXISTS app_skills (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL UNIQUE,
          description TEXT NOT NULL DEFAULT '',
          content TEXT NOT NULL DEFAULT '',
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS project_agent_runtime_configs (
          project_id TEXT NOT NULL,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          config_options_json TEXT NOT NULL DEFAULT '{}',
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(project_id, role_name, runtime_kind)
        );
        CREATE TABLE IF NOT EXISTS app_session_role_runtime_configs (
          app_session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          runtime_profile_id TEXT,
          acp_session_id TEXT,
          model_override TEXT,
          mode_override TEXT,
          config_options_json TEXT NOT NULL DEFAULT '{}',
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(app_session_id, role_name, runtime_kind)
        );
        CREATE INDEX IF NOT EXISTS idx_app_session_role_runtime_configs_lookup
          ON app_session_role_runtime_configs(app_session_id, role_name, runtime_kind);
        CREATE TABLE IF NOT EXISTS app_session_messages (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
          role_name TEXT NOT NULL,
          content TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_inbox_messages (
          id TEXT PRIMARY KEY,
          app_session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
          role_name TEXT,
          delivery TEXT NOT NULL DEFAULT 'nextTurn',
          text TEXT NOT NULL,
          attachments_json TEXT NOT NULL DEFAULT '[]',
          created_at INTEGER NOT NULL,
          claimed_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_lifecycle (
          app_session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          state TEXT NOT NULL,
          revision INTEGER NOT NULL DEFAULT 0,
          last_error TEXT,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(app_session_id, role_name, runtime_kind)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_session
          ON agent_lifecycle(app_session_id, updated_at DESC);

        CREATE INDEX IF NOT EXISTS idx_roles_updated_at
          ON roles(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_workflows_updated_at
          ON workflows(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_created_at
          ON sessions(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_workflow_id
          ON sessions(workflow_id);
        CREATE INDEX IF NOT EXISTS idx_session_events_session_id_id
          ON session_events(session_id, id ASC);
        CREATE INDEX IF NOT EXISTS idx_shared_context_scope_updated_at
          ON shared_context_snapshots(scope, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_dynamic_catalog_kind_updated_at
          ON dynamic_catalog_entries(kind, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_app_sessions_last_active
          ON app_sessions(last_active_at DESC);
        CREATE INDEX IF NOT EXISTS idx_app_skills_updated_at
          ON app_skills(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_app_session_messages_session_id
          ON app_session_messages(session_id, id ASC);
        CREATE INDEX IF NOT EXISTS idx_session_inbox_pending
          ON session_inbox_messages(app_session_id, claimed_at, created_at, id);

        CREATE TABLE IF NOT EXISTS global_mcp_servers (
          name TEXT PRIMARY KEY,
          config_json TEXT NOT NULL,
          is_builtin INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS runtime_profiles (
          id TEXT PRIMARY KEY,
          label TEXT NOT NULL,
          command TEXT NOT NULL,
          args_json TEXT NOT NULL DEFAULT '[]',
          env_refs_json TEXT NOT NULL DEFAULT '[]',
          cwd_strategy TEXT NOT NULL DEFAULT 'session',
          capabilities_json TEXT NOT NULL DEFAULT '{}',
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS role_mcp_servers (
          role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
          mcp_server_name TEXT NOT NULL REFERENCES global_mcp_servers(name) ON DELETE CASCADE,
          enabled INTEGER NOT NULL DEFAULT 1,
          PRIMARY KEY(role_id, mcp_server_name)
        );

        CREATE TABLE IF NOT EXISTS rules (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL UNIQUE,
          content TEXT NOT NULL,
          description TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS role_rules (
          role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
          rule_id TEXT NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
          enabled INTEGER NOT NULL DEFAULT 1,
          ord INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY(role_id, rule_id)
        );

        CREATE TABLE IF NOT EXISTS projects (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          root_path TEXT NOT NULL UNIQUE,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_projects_updated_at ON projects(updated_at DESC);

        CREATE TABLE IF NOT EXISTS role_skills (
          role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
          skill_id TEXT NOT NULL REFERENCES app_skills(id) ON DELETE CASCADE,
          enabled INTEGER NOT NULL DEFAULT 1,
          ord INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY(role_id, skill_id)
        );

        CREATE TABLE IF NOT EXISTS project_agent_configs (
          project_id TEXT NOT NULL DEFAULT '',
          role_name TEXT NOT NULL,
          runtime_kind TEXT,
          config_options_json TEXT NOT NULL DEFAULT '{}',
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(project_id, role_name)
        );
        CREATE TABLE IF NOT EXISTS project_role_overrides (
          project_id TEXT NOT NULL,
          role_name TEXT NOT NULL,
          overrides_json TEXT NOT NULL DEFAULT '{}',
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(project_id, role_name)
        );
        CREATE INDEX IF NOT EXISTS idx_project_role_overrides_project
          ON project_role_overrides(project_id, role_name);

        CREATE TABLE IF NOT EXISTS import_session_catalog (
          project_id TEXT NOT NULL,
          source_id TEXT NOT NULL,
          external_session_id TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          agent TEXT NOT NULL,
          cwd TEXT NOT NULL,
          title TEXT NOT NULL,
          turn_count INTEGER NOT NULL,
          created_at INTEGER NOT NULL,
          last_active_at INTEGER NOT NULL,
          file_path TEXT NOT NULL,
          scanned_at INTEGER NOT NULL,
          PRIMARY KEY(project_id, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_import_session_catalog_project_scan
          ON import_session_catalog(project_id, scanned_at DESC);
        CREATE TABLE IF NOT EXISTS import_session_catalog_scans (
          project_id TEXT PRIMARY KEY,
          scanned_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_rules_name ON rules(name ASC);
        ",
    )
    .map_err(|e| e.to_string())?;
    ensure_column(
        conn,
        "roles",
        "config_option_defs_json",
        "TEXT DEFAULT '[]'",
    )?;
    ensure_column(conn, "roles", "runtime_profile_id", "TEXT")?;
    ensure_column(conn, "roles", "project_id", "TEXT")?;
    ensure_column(conn, "app_sessions", "runtime_profile_id", "TEXT")?;
    ensure_column(conn, "app_sessions", "project_id", "TEXT")?;
    ensure_column(conn, "projects", "deleted_at", "INTEGER")?;
    ensure_column(conn, "app_sessions", "external_session_id", "TEXT")?;
    ensure_column(conn, "app_session_roles", "runtime_profile_id", "TEXT")?;
    ensure_column(conn, "app_session_roles", "model_override", "TEXT")?;
    ensure_column(conn, "app_session_roles", "mode_override", "TEXT")?;
    ensure_column(conn, "app_session_roles", "mcp_servers_json", "TEXT")?;
    ensure_column(conn, "app_session_roles", "config_options_json", "TEXT")?;
    // `content` stays the always-populated plain-text projection (MCP's get_session_history
    // reads it directly and must keep working unchanged); `payload` holds the structured
    // extras (tool calls, diffs, plan, images, thought) as JSON when content_type = 'json',
    // so a turn's rich content survives reload instead of collapsing to bare text.
    ensure_column(
        conn,
        "app_session_messages",
        "content_type",
        "TEXT NOT NULL DEFAULT 'text'",
    )?;
    ensure_column(conn, "app_session_messages", "payload", "TEXT")?;
    ensure_column(conn, "app_session_messages", "client_id", "TEXT")?;

    // Indexed after the column additions above, because `external_session_id` is itself one
    // of them and a fresh database has no such column while the schema batch runs.
    conn.execute_batch(
        "
        -- The session list filters on closed_at before sorting, so an index on the sort
        -- column alone still scans every closed session.
        CREATE INDEX IF NOT EXISTS idx_app_sessions_open_last_active
          ON app_sessions(closed_at, last_active_at DESC);
        -- Looked up once per transcript found on disk during an import scan.
        CREATE INDEX IF NOT EXISTS idx_app_sessions_external_session_id
          ON app_sessions(external_session_id);
        CREATE INDEX IF NOT EXISTS idx_app_sessions_runtime_external_session_id
          ON app_sessions(runtime_kind, external_session_id);
        -- The primary key leads with the session id, so role-wide lookups (deleting a
        -- persona, listing the sessions bound to one) cannot use it.
        CREATE INDEX IF NOT EXISTS idx_app_session_roles_role_name
          ON app_session_roles(role_name);
        CREATE INDEX IF NOT EXISTS idx_app_session_messages_session_client_id
          ON app_session_messages(session_id, client_id) WHERE client_id IS NOT NULL;
        ",
    )
    .map_err(|e| e.to_string())?;

    // Model catalog entries used to be keyed by (kind, name) only, so a model picked in a
    // Codex role became visible to every runtime. Re-key by runtime and attribute the
    // existing rows by model family; anything unattributable stays runtime-less and is
    // surfaced only as a fallback for runtimes with no discovered catalog.
    let catalog_schema: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='dynamic_catalog_entries'",
            [],
            |r| r.get(0),
        )
        .ok();
    if catalog_schema
        .map(|sql| !sql.contains("runtime_key"))
        .unwrap_or(false)
    {
        let _ = conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS dynamic_catalog_migrated (
              kind TEXT NOT NULL,
              runtime_key TEXT NOT NULL DEFAULT '',
              name TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(kind, runtime_key, name)
            );
            INSERT OR IGNORE INTO dynamic_catalog_migrated (kind, runtime_key, name, created_at, updated_at)
            SELECT kind,
                   CASE
                     WHEN kind <> 'model' THEN ''
                     WHEN lower(name) LIKE 'claude-%' THEN 'claude-native'
                     WHEN lower(name) IN ('sonnet','opus','haiku','fable') THEN 'claude-native'
                     WHEN lower(name) LIKE 'gpt-%' OR lower(name) LIKE 'o1%' OR lower(name) LIKE 'o3%' OR lower(name) LIKE 'codex%' THEN 'codex-cli'
                     WHEN lower(name) LIKE 'gemini-%' THEN 'antigravity-cli'
                     ELSE ''
                   END,
                   name, created_at, updated_at FROM dynamic_catalog_entries;
            DROP TABLE dynamic_catalog_entries;
            ALTER TABLE dynamic_catalog_migrated RENAME TO dynamic_catalog_entries;
            ",
        );
    }

    // The agent layer was first keyed by (project, runtime), which meant two personas on the
    // same CLI shared one model/effort and overwrote each other. Re-key by (project, persona):
    // the persona is what the user actually switches between, and its own runtime_kind/model
    // become the seed for a project that has not tuned it yet.
    let agent_schema: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='project_agent_configs'",
            [],
            |r| r.get(0),
        )
        .ok();
    if agent_schema
        .map(|sql| !sql.contains("role_name"))
        .unwrap_or(false)
    {
        let _ = conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS project_agent_configs_migrated (
              project_id TEXT NOT NULL DEFAULT '',
              role_name TEXT NOT NULL,
              runtime_kind TEXT,
              config_options_json TEXT NOT NULL DEFAULT '{}',
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(project_id, role_name)
            );
            -- Attribute each old row to the role that was most recently active on that
            -- runtime in that project. Rows with no matching session are dropped: guessing a
            -- persona would silently hand one persona another's model.
            INSERT OR IGNORE INTO project_agent_configs_migrated
              (project_id, role_name, runtime_kind, config_options_json, updated_at)
            SELECT old.project_id,
                   (SELECT s.active_role FROM app_sessions s
                      WHERE IFNULL(s.project_id,'') = old.project_id
                        AND IFNULL(s.runtime_kind,'') = old.runtime_key
                      ORDER BY s.last_active_at DESC LIMIT 1),
                   old.runtime_key, old.config_options_json, old.updated_at
            FROM project_agent_configs old
            WHERE (SELECT s.active_role FROM app_sessions s
                     WHERE IFNULL(s.project_id,'') = old.project_id
                       AND IFNULL(s.runtime_kind,'') = old.runtime_key
                     ORDER BY s.last_active_at DESC LIMIT 1) IS NOT NULL;
            DROP TABLE project_agent_configs;
            ALTER TABLE project_agent_configs_migrated RENAME TO project_agent_configs;
            ",
        );
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_project_agent_configs_project_role
           ON project_agent_configs(project_id, updated_at DESC);",
    )
    .map_err(|e| e.to_string())?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_dynamic_catalog_kind_runtime_updated_at
           ON dynamic_catalog_entries(kind, runtime_key, updated_at DESC);",
    )
    .map_err(|e| e.to_string())?;

    let roles_schema: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='roles'",
            [],
            |r| r.get(0),
        )
        .ok();
    if let Some(sql) = roles_schema {
        if sql.contains("role_name TEXT NOT NULL UNIQUE") || sql.contains("role_name TEXT UNIQUE") {
            let _ = conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS roles_migrated (
                  id TEXT PRIMARY KEY,
                  role_name TEXT NOT NULL,
                  runtime_kind TEXT NOT NULL,
                  runtime_profile_id TEXT,
                  system_prompt TEXT NOT NULL,
                  model TEXT,
                  mode TEXT,
                  mcp_servers_json TEXT DEFAULT '[]',
                  config_options_json TEXT DEFAULT '{}',
                  config_option_defs_json TEXT DEFAULT '[]',
                  auto_approve INTEGER DEFAULT 1,
                  project_id TEXT,
                  created_at INTEGER NOT NULL,
                  updated_at INTEGER NOT NULL
                );
                INSERT OR IGNORE INTO roles_migrated (id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at)
                SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at FROM roles;
                DROP TABLE roles;
                ALTER TABLE roles_migrated RENAME TO roles;
                ",
            );
        }
    }

    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_roles_project_id ON roles(project_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_roles_name_project ON roles(role_name, IFNULL(project_id, ''));
        CREATE INDEX IF NOT EXISTS idx_app_sessions_project_id ON app_sessions(project_id);
        ",
    )
    .map_err(|e| e.to_string())?;

    // One-shot data migrations, each gated on the version that introduced it so it runs
    // exactly once. Ungated, they re-ran on every launch and fought the user's own edits —
    // one of them silently moved every Codex session back to Claude on each start.
    if from_version < 11 {
        let profile_case = "CASE lower(trim(runtime_kind))
            WHEN 'claude-code' THEN 'acp:claude-code'
            WHEN 'claude-acp' THEN 'acp:claude-code'
            WHEN 'acp:claude-code' THEN 'acp:claude-code'
            WHEN 'claude' THEN 'native:claude'
            WHEN 'claude-native' THEN 'native:claude'
            WHEN 'native:claude' THEN 'native:claude'
            WHEN 'antigravity-cli' THEN 'native:agy'
            WHEN 'agy' THEN 'native:agy'
            WHEN 'gemini' THEN 'native:agy'
            WHEN 'gemini-cli' THEN 'native:agy'
            WHEN 'native:agy' THEN 'native:agy'
            WHEN 'codex-cli' THEN 'native:codex'
            WHEN 'codex' THEN 'native:codex'
            WHEN 'native:codex' THEN 'native:codex'
            WHEN 'pi-cli' THEN 'native:pi'
            WHEN 'pi' THEN 'native:pi'
            WHEN 'native:pi' THEN 'native:pi'
            ELSE lower(trim(runtime_kind)) END";
        for table in ["roles", "app_sessions", "app_session_roles"] {
            conn.execute(
                &format!(
                    "UPDATE {table} SET runtime_profile_id = {profile_case}
                     WHERE runtime_profile_id IS NULL OR trim(runtime_profile_id) = ''"
                ),
                [],
            )
            .map_err(|e| e.to_string())?;
        }

        let runtime_case = "CASE lower(trim(runtime_kind))
            WHEN 'claude-code' THEN 'claude-code'
            WHEN 'claude-acp' THEN 'claude-code'
            WHEN 'acp:claude-code' THEN 'claude-code'
            WHEN 'claude' THEN 'claude-native'
            WHEN 'claude-native' THEN 'claude-native'
            WHEN 'native:claude' THEN 'claude-native'
            WHEN 'antigravity-cli' THEN 'antigravity-cli'
            WHEN 'agy' THEN 'antigravity-cli'
            WHEN 'gemini' THEN 'antigravity-cli'
            WHEN 'gemini-cli' THEN 'antigravity-cli'
            WHEN 'native:agy' THEN 'antigravity-cli'
            WHEN 'codex-cli' THEN 'codex-cli'
            WHEN 'codex' THEN 'codex-cli'
            WHEN 'native:codex' THEN 'codex-cli'
            WHEN 'pi-cli' THEN 'pi-cli'
            WHEN 'pi' THEN 'pi-cli'
            WHEN 'native:pi' THEN 'pi-cli'
            ELSE runtime_kind END";
        for table in ["roles", "app_sessions", "app_session_roles"] {
            conn.execute(
                &format!(
                    "UPDATE {table} SET runtime_kind = {runtime_case}
                     WHERE runtime_kind IS NOT NULL"
                ),
                [],
            )
            .map_err(|e| e.to_string())?;
        }

        conn.execute(
            "UPDATE app_sessions SET active_role = 'Developer' WHERE active_role = 'Jockey'",
            [],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE roles
             SET system_prompt = ''
             WHERE system_prompt = 'You are a senior developer. Implement the solution step by step.'
                OR system_prompt = 'You are a helpful AI assistant.'",
            [],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE app_sessions
             SET project_id = (
                 SELECT p.id FROM projects p
                 WHERE app_sessions.cwd = p.root_path
                    OR app_sessions.cwd LIKE p.root_path || '/%'
                 ORDER BY length(p.root_path) DESC
                 LIMIT 1
             )
             WHERE EXISTS (
                 SELECT 1 FROM projects p
                 WHERE app_sessions.cwd = p.root_path
                    OR app_sessions.cwd LIKE p.root_path || '/%'
             )",
            [],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE app_sessions
             SET project_id = (SELECT id FROM projects ORDER BY updated_at DESC LIMIT 1)
             WHERE (project_id IS NULL OR trim(project_id) = '')
               AND (SELECT count(1) FROM projects) > 0",
            [],
        )
        .map_err(|e| e.to_string())?;

        // NOTE: two one-off dev fixups used to live here, rewriting every `codex-cli` row in
        // `app_sessions` (and the `developer_copy` role) to `claude-native`. They were NOT
        // version-gated, so they re-ran on every startup and silently destroyed the user's engine
        // choice: no session could stay on Codex across a restart. Removed. Anything re-added
        // here must be gated on `user_version` so it runs exactly once.
    }

    if from_version < 12 {
        // Knobs used to be stored per (project, persona) and wiped whenever the engine
        // changed. They now live per (project, persona, engine); move each existing set onto
        // the engine it was actually configured for so nobody's current setup is lost.
        conn.execute(
            "INSERT OR IGNORE INTO project_agent_runtime_configs
                 (project_id, role_name, runtime_kind, config_options_json, updated_at)
             SELECT project_id, role_name, runtime_kind, config_options_json, updated_at
             FROM project_agent_configs
             WHERE runtime_kind IS NOT NULL
               AND trim(runtime_kind) <> ''
               AND config_options_json IS NOT NULL
               AND trim(config_options_json) NOT IN ('', '{}')",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    let developer_exists: i64 = conn
        .query_row(
            "SELECT count(1) FROM roles WHERE lower(role_name) = 'developer'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if developer_exists == 0 {
        let dev_id = uuid::Uuid::new_v4().to_string();
        let now = crate::now_ms();
        conn.execute(
            "INSERT INTO roles (id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at)
             VALUES (?1, 'Developer', 'claude-native', 'native:claude', '', NULL, NULL, '[]', '{}', '[]', 1, NULL, ?2, ?2)",
            rusqlite::params![dev_id, now],
        )
        .map_err(|e| e.to_string())?;
    }

    if from_version < 13 && table_has_column(conn, "role_mcp_servers", "role_name")? {
        conn.execute_batch(
            "
            CREATE TABLE role_mcp_servers_v13 (
              role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
              mcp_server_name TEXT NOT NULL REFERENCES global_mcp_servers(name) ON DELETE CASCADE,
              enabled INTEGER NOT NULL DEFAULT 1,
              PRIMARY KEY(role_id, mcp_server_name)
            );
            INSERT OR IGNORE INTO role_mcp_servers_v13 (role_id, mcp_server_name, enabled)
            SELECT (
              SELECT id FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
              ORDER BY updated_at DESC LIMIT 1
            ), legacy.mcp_server_name, legacy.enabled
            FROM role_mcp_servers legacy
            WHERE EXISTS (
              SELECT 1 FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
            );
            DROP TABLE role_mcp_servers;
            ALTER TABLE role_mcp_servers_v13 RENAME TO role_mcp_servers;
            CREATE INDEX idx_role_mcp_servers_role ON role_mcp_servers(role_id);
            ",
        )
        .map_err(|e| e.to_string())?;
    }
    if from_version < 13 && table_has_column(conn, "role_rules", "role_name")? {
        conn.execute_batch(
            "
            CREATE TABLE role_rules_v13 (
              role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
              rule_id TEXT NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
              enabled INTEGER NOT NULL DEFAULT 1,
              ord INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY(role_id, rule_id)
            );
            INSERT OR IGNORE INTO role_rules_v13 (role_id, rule_id, enabled, ord)
            SELECT (
              SELECT id FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
              ORDER BY updated_at DESC LIMIT 1
            ), legacy.rule_id, legacy.enabled, legacy.ord
            FROM role_rules legacy
            WHERE EXISTS (
              SELECT 1 FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
            );
            DROP TABLE role_rules;
            ALTER TABLE role_rules_v13 RENAME TO role_rules;
            CREATE INDEX idx_role_rules_role ON role_rules(role_id, ord ASC);
            ",
        )
        .map_err(|e| e.to_string())?;
    }
    if from_version < 13 && table_has_column(conn, "role_skills", "role_name")? {
        conn.execute_batch(
            "
            CREATE TABLE role_skills_v13 (
              role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
              skill_id TEXT NOT NULL REFERENCES app_skills(id) ON DELETE CASCADE,
              enabled INTEGER NOT NULL DEFAULT 1,
              ord INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY(role_id, skill_id)
            );
            INSERT OR IGNORE INTO role_skills_v13 (role_id, skill_id, enabled, ord)
            SELECT (
              SELECT id FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
              ORDER BY updated_at DESC LIMIT 1
            ), legacy.skill_id, legacy.enabled, legacy.ord
            FROM role_skills legacy
            WHERE EXISTS (
              SELECT 1 FROM roles
              WHERE lower(role_name) = lower(legacy.role_name)
                AND (project_id IS NULL OR project_id = '')
            );
            DROP TABLE role_skills;
            ALTER TABLE role_skills_v13 RENAME TO role_skills;
            CREATE INDEX idx_role_skills_role ON role_skills(role_id, ord ASC);
            ",
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_role_mcp_servers_role ON role_mcp_servers(role_id);
        CREATE INDEX IF NOT EXISTS idx_role_rules_role ON role_rules(role_id, ord ASC);
        CREATE INDEX IF NOT EXISTS idx_role_skills_role ON role_skills(role_id, ord ASC);
        ",
    )
    .map_err(|e| e.to_string())?;

    if from_version < 14 {
        conn.execute(
            "INSERT OR IGNORE INTO app_session_role_runtime_configs
                 (app_session_id, role_name, runtime_kind, runtime_profile_id, acp_session_id, model_override, mode_override, config_options_json, updated_at)
             SELECT app_session_id, role_name, runtime_kind, runtime_profile_id, acp_session_id, model_override, mode_override, COALESCE(config_options_json, '{}'), ?1
             FROM app_session_roles
             WHERE runtime_kind IS NOT NULL AND trim(runtime_kind) <> ''",
            rusqlite::params![crate::now_ms()],
        )
        .map_err(|e| e.to_string())?;
    }

    if from_version < 15 {
        conn.execute_batch(
            "
            DELETE FROM import_session_catalog;
            DELETE FROM import_session_catalog_scans;
            ",
        )
        .map_err(|e| e.to_string())?;
    }

    if from_version < 16 {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_projects_active_root_path
             ON projects(root_path, deleted_at);",
        )
        .map_err(|e| e.to_string())?;
    }

    // Only stamped once everything above succeeded, so a failed migration is retried on the
    // next launch instead of being silently marked as applied.
    if from_version != SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| e.to_string())?;
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
    let conn = state.db.get().map_err(|e| e.to_string())?;
    f(&conn)
}

pub(crate) fn parse_payload(payload: String) -> Value {
    serde_json::from_str::<Value>(&payload).unwrap_or(json!({ "text": payload }))
}

pub(crate) fn seed_default_dynamic_catalog(state: &AppState) -> Result<(), String> {
    for model in DEFAULT_MODELS {
        let _ = context::upsert_dynamic_catalog_item(state, "model", "", model)?;
    }
    for server in DEFAULT_MCP_SERVERS {
        let _ = context::upsert_dynamic_catalog_item(state, "mcp", "", server)?;
    }
    for skill in DEFAULT_SKILLS {
        let _ = context::upsert_dynamic_catalog_item(state, "skill", "", skill)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::init_db;
    use rusqlite::Connection;

    fn legacy_catalog_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "
            CREATE TABLE dynamic_catalog_entries (
              kind TEXT NOT NULL,
              name TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(kind, name)
            );
            INSERT INTO dynamic_catalog_entries VALUES
              ('model','gpt-5.6-sol',1,1),
              ('model','claude-sonnet-5',1,1),
              ('model','sonnet',1,1),
              ('model','gemini-3.8-flash-high',1,1),
              ('model','some-local-llm',1,1),
              ('mcp','chrome-devtools',1,1);
            ",
        )
        .expect("seed legacy rows");
        conn
    }

    fn names_for(conn: &Connection, kind: &str, runtime_key: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM dynamic_catalog_entries
                 WHERE kind = ?1 AND (?2 = '' OR runtime_key IN ('', ?2))
                 ORDER BY name ASC",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(rusqlite::params![kind, runtime_key], |row| {
                row.get::<_, String>(0)
            })
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }

    #[test]
    fn migration_attributes_legacy_models_to_their_runtime() {
        let conn = legacy_catalog_db();
        init_db(&conn).expect("init_db");

        // The pollution this migration exists to stop: a Codex pick must not reach Claude.
        let claude = names_for(&conn, "model", "claude-native");
        assert!(claude.contains(&"claude-sonnet-5".to_string()));
        assert!(claude.contains(&"sonnet".to_string()));
        assert!(!claude.contains(&"gpt-5.6-sol".to_string()));
        assert!(!claude.contains(&"gemini-3.8-flash-high".to_string()));

        let codex = names_for(&conn, "model", "codex-cli");
        assert!(codex.contains(&"gpt-5.6-sol".to_string()));
        assert!(!codex.contains(&"claude-sonnet-5".to_string()));

        // Unattributable rows stay runtime-less and remain visible everywhere.
        assert!(claude.contains(&"some-local-llm".to_string()));
        assert!(codex.contains(&"some-local-llm".to_string()));
    }

    #[test]
    fn fresh_install_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("first init");
        init_db(&conn).expect("re-init must not fail");
        assert!(names_for(&conn, "model", "claude-native").is_empty());
    }

    #[test]
    fn re_init_over_migrated_db_is_stable() {
        let conn = legacy_catalog_db();
        init_db(&conn).expect("migrate");
        init_db(&conn).expect("re-init after migration");
        assert!(names_for(&conn, "model", "codex-cli").contains(&"gpt-5.6-sol".to_string()));
    }

    fn legacy_message_db() -> Connection {
        // Build the full current schema first (so every other table/column this migration
        // doesn't own is already correct), then downgrade just app_session_messages to its
        // pre-migration shape — this is what an existing user's database actually looks like.
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("init fresh schema");
        conn.execute_batch(
            "
            INSERT INTO app_sessions (id, title, active_role, created_at, last_active_at)
              VALUES ('s1', 'Session_1', 'Developer', 1, 1);
            DROP TABLE app_session_messages;
            CREATE TABLE app_session_messages (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL REFERENCES app_sessions(id) ON DELETE CASCADE,
              role_name TEXT NOT NULL,
              content TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            INSERT INTO app_session_messages (session_id, role_name, content, created_at)
              VALUES ('s1', 'user', 'hello', 1);
            ",
        )
        .expect("simulate pre-migration app_session_messages");
        conn
    }

    #[test]
    fn message_schema_migration_backfills_content_type_for_free() {
        // Pre-migration rows have no content_type/payload columns at all; a plain
        // `ALTER TABLE ... ADD COLUMN ... DEFAULT` must backfill them with no manual rewrite,
        // and the old plain-text row must stay readable exactly as it was.
        let conn = legacy_message_db();
        init_db(&conn).expect("migrate");
        let (content, content_type, payload): (String, String, Option<String>) = conn
            .query_row(
                "SELECT content, content_type, payload FROM app_session_messages WHERE session_id = 's1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("row");
        assert_eq!(content, "hello");
        assert_eq!(content_type, "text");
        assert!(payload.is_none());
    }

    #[test]
    fn hot_lookups_are_index_backed_rather_than_full_scans() {
        // Asserting the plan, not just the index's existence: an index on the sort column
        // alone still scanned every closed session, which is the bug that prompted these.
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("init");

        let plan = |sql: &str| -> String {
            let mut stmt = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .expect("prepare");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(3))
                .expect("plan")
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            rows.join(" | ")
        };

        let open_sessions = plan(
            "SELECT id FROM app_sessions WHERE closed_at IS NULL ORDER BY last_active_at DESC LIMIT 100",
        );
        assert!(
            open_sessions.contains("idx_app_sessions_open_last_active"),
            "session list should not scan closed sessions: {open_sessions}"
        );

        let by_external = plan("SELECT id FROM app_sessions WHERE external_session_id = 'x'");
        assert!(
            by_external.contains("idx_app_sessions_external_session_id"),
            "import scan should not full-scan sessions: {by_external}"
        );

        let by_role = plan("SELECT app_session_id FROM app_session_roles WHERE role_name = 'Dev'");
        assert!(
            by_role.contains("idx_app_session_roles_role_name"),
            "role-wide lookup cannot use a PK that leads with the session id: {by_role}"
        );
    }

    #[test]
    fn one_shot_migrations_do_not_re_run_and_cannot_overwrite_later_edits() {
        // The bug this guards: an ungated fixup re-ran on every launch and rewrote the user's
        // own choices — a session moved to Codex was silently moved back to Claude each start.
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("first init");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("user_version");
        assert_eq!(version, super::SCHEMA_VERSION);

        conn.execute(
            "INSERT INTO app_sessions (id, title, active_role, runtime_kind, created_at, last_active_at)
             VALUES ('s1', 'Session_1', 'Jockey', 'codex-cli', 1, 1)",
            [],
        )
        .expect("insert session the user just re-pointed at Codex");

        init_db(&conn).expect("re-init");

        let (role, runtime): (String, String) = conn
            .query_row(
                "SELECT active_role, runtime_kind FROM app_sessions WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row");
        assert_eq!(runtime, "codex-cli", "re-init must not rewrite the engine");
        assert_eq!(role, "Jockey", "re-init must not rewrite the persona");
    }

    #[test]
    fn migration_leaves_global_kinds_unscoped() {
        let conn = legacy_catalog_db();
        init_db(&conn).expect("init_db");
        for runtime in ["claude-native", "codex-cli", "antigravity-cli"] {
            assert_eq!(
                names_for(&conn, "mcp", runtime),
                vec!["chrome-devtools".to_string()]
            );
        }
    }

    #[test]
    fn migration_scopes_legacy_role_bindings_to_the_global_role() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("initialize current schema");
        conn.execute(
            "INSERT INTO global_mcp_servers (name, config_json, is_builtin, created_at, updated_at)
             VALUES ('test-server', '{}', 0, 1, 1)",
            [],
        )
        .expect("insert mcp server");
        let global_role_id: String = conn
            .query_row(
                "SELECT id FROM roles WHERE role_name = 'Developer'
                 AND COALESCE(project_id, '') = ''",
                [],
                |row| row.get(0),
            )
            .expect("global Developer role");

        conn.execute_batch(
            "DROP TABLE role_mcp_servers;
             CREATE TABLE role_mcp_servers (
               role_name TEXT NOT NULL,
               mcp_server_name TEXT NOT NULL,
               enabled INTEGER NOT NULL DEFAULT 1,
               PRIMARY KEY(role_name, mcp_server_name)
             );
             INSERT INTO role_mcp_servers (role_name, mcp_server_name, enabled)
             VALUES ('Developer', 'test-server', 1);
             PRAGMA user_version = 12;",
        )
        .expect("install legacy table");

        init_db(&conn).expect("migrate legacy bindings");
        let migrated_role_id: String = conn
            .query_row(
                "SELECT role_id FROM role_mcp_servers WHERE mcp_server_name = 'test-server'",
                [],
                |row| row.get(0),
            )
            .expect("migrated role binding");
        assert_eq!(migrated_role_id, global_role_id);
    }
}
