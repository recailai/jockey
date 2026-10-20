use super::provider::CredentialProvider;
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS agent_credentials (
          id TEXT PRIMARY KEY,
          provider TEXT NOT NULL,
          label TEXT NOT NULL,
          config_dir TEXT,
          managed INTEGER NOT NULL DEFAULT 0,
          env_keys_json TEXT NOT NULL DEFAULT '[]',
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS role_credentials (
          role_id TEXT NOT NULL,
          provider TEXT NOT NULL,
          credential_id TEXT NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(role_id, provider)
        );
        ",
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct AgentCredential {
    pub(crate) id: String,
    pub(crate) provider: CredentialProvider,
    pub(crate) label: String,
    /// `None` keeps the CLI's own default dir (its current login).
    pub(crate) config_dir: Option<String>,
    /// The dir was created by Jockey under its data dir and may be removed with the profile.
    pub(crate) managed: bool,
    /// Names only; the values are secrets and live in the keychain (see `secrets`).
    pub(crate) env_keys: Vec<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

const COLUMNS: &str =
    "id, provider, label, config_dir, managed, env_keys_json, created_at, updated_at";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Option<AgentCredential>> {
    let provider: String = row.get(1)?;
    let env_keys_json: String = row.get(5)?;
    Ok(
        CredentialProvider::from_key(&provider).map(|provider| AgentCredential {
            id: row.get(0).unwrap_or_default(),
            provider,
            label: row.get(2).unwrap_or_default(),
            config_dir: row.get(3).ok().flatten(),
            managed: row.get::<_, i64>(4).unwrap_or(0) != 0,
            env_keys: serde_json::from_str(&env_keys_json).unwrap_or_default(),
            created_at: row.get(6).unwrap_or_default(),
            updated_at: row.get(7).unwrap_or_default(),
        }),
    )
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<AgentCredential>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM agent_credentials ORDER BY provider, created_at"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], from_row).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok().flatten()).collect())
}

pub(crate) fn get(conn: &Connection, id: &str) -> Result<Option<AgentCredential>, String> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM agent_credentials WHERE id = ?1"),
        params![id],
        from_row,
    )
    .optional()
    .map(Option::flatten)
    .map_err(|e| e.to_string())
}

pub(crate) fn upsert(conn: &Connection, c: &AgentCredential) -> Result<(), String> {
    let env_keys_json = serde_json::to_string(&c.env_keys).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO agent_credentials (id, provider, label, config_dir, managed, env_keys_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           label = excluded.label,
           config_dir = excluded.config_dir,
           env_keys_json = excluded.env_keys_json,
           updated_at = excluded.updated_at",
        params![
            c.id,
            c.provider.key(),
            c.label,
            c.config_dir,
            c.managed as i64,
            env_keys_json,
            c.created_at,
            c.updated_at
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub(crate) fn delete(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM role_credentials WHERE credential_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM agent_credentials WHERE id = ?1", params![id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub(crate) fn bound_credential_id(
    conn: &Connection,
    role_id: &str,
    provider: CredentialProvider,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT credential_id FROM role_credentials WHERE role_id = ?1 AND provider = ?2",
        params![role_id, provider.key()],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub(crate) fn role_bindings(
    conn: &Connection,
    role_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT provider, credential_id FROM role_credentials WHERE role_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![role_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Role names per credential, for the "used by" column.
pub(crate) fn bound_role_names(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT rc.credential_id, r.role_name
             FROM role_credentials rc JOIN roles r ON r.id = rc.role_id
             ORDER BY r.role_name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub(crate) fn set_binding(
    conn: &Connection,
    role_id: &str,
    provider: CredentialProvider,
    credential_id: Option<&str>,
    now: i64,
) -> Result<(), String> {
    match credential_id {
        Some(credential_id) => conn.execute(
            "INSERT INTO role_credentials (role_id, provider, credential_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(role_id, provider) DO UPDATE SET
               credential_id = excluded.credential_id,
               updated_at = excluded.updated_at",
            params![role_id, provider.key(), credential_id, now],
        ),
        None => conn.execute(
            "DELETE FROM role_credentials WHERE role_id = ?1 AND provider = ?2",
            params![role_id, provider.key()],
        ),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub(crate) fn delete_role_bindings(conn: &Connection, role_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM role_credentials WHERE role_id = ?1",
        params![role_id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}
