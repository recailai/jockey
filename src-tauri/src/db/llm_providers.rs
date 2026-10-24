//! SQLite rows for per-runtime LLM endpoint configs. Only header names are stored; header
//! values live in the OS keychain (see `crate::llm_providers`).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredConfig {
    pub(crate) provider_id: String,
    pub(crate) api_type: String,
    pub(crate) base_url: String,
    pub(crate) header_names: Vec<String>,
}

pub(crate) fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_provider_configs (
           runtime_key TEXT NOT NULL,
           provider_id TEXT NOT NULL,
           api_type TEXT NOT NULL,
           base_url TEXT NOT NULL,
           header_names_json TEXT NOT NULL DEFAULT '[]',
           updated_at INTEGER NOT NULL,
           PRIMARY KEY(runtime_key, provider_id)
         );",
    )
    .map_err(|e| e.to_string())
}

fn from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<StoredConfig> {
    let names: String = row.get(offset + 3)?;
    Ok(StoredConfig {
        provider_id: row.get(offset)?,
        api_type: row.get(offset + 1)?,
        base_url: row.get(offset + 2)?,
        header_names: serde_json::from_str(&names).unwrap_or_default(),
    })
}

pub(crate) fn read_all(conn: &Connection) -> Result<HashMap<String, Vec<StoredConfig>>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT runtime_key, provider_id, api_type, base_url, header_names_json
             FROM llm_provider_configs ORDER BY runtime_key, provider_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, from_row(row, 1)?)))
        .map_err(|e| e.to_string())?;
    let mut out: HashMap<String, Vec<StoredConfig>> = HashMap::new();
    for row in rows {
        let (runtime, cfg) = row.map_err(|e| e.to_string())?;
        out.entry(runtime).or_default().push(cfg);
    }
    Ok(out)
}

pub(crate) fn read_runtime(
    conn: &Connection,
    runtime_key: &str,
) -> Result<Vec<StoredConfig>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT provider_id, api_type, base_url, header_names_json
             FROM llm_provider_configs WHERE runtime_key = ?1 ORDER BY provider_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![runtime_key], |row| from_row(row, 0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub(crate) fn read_one(
    conn: &Connection,
    runtime_key: &str,
    provider_id: &str,
) -> Result<Option<StoredConfig>, String> {
    conn.query_row(
        "SELECT provider_id, api_type, base_url, header_names_json
         FROM llm_provider_configs WHERE runtime_key = ?1 AND provider_id = ?2",
        params![runtime_key, provider_id],
        |row| from_row(row, 0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub(crate) fn upsert(
    conn: &Connection,
    runtime_key: &str,
    cfg: &StoredConfig,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO llm_provider_configs
           (runtime_key, provider_id, api_type, base_url, header_names_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))
         ON CONFLICT(runtime_key, provider_id) DO UPDATE SET
           api_type = excluded.api_type, base_url = excluded.base_url,
           header_names_json = excluded.header_names_json, updated_at = excluded.updated_at",
        params![
            runtime_key,
            cfg.provider_id,
            cfg.api_type,
            cfg.base_url,
            serde_json::to_string(&cfg.header_names).map_err(|e| e.to_string())?
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn delete(
    conn: &Connection,
    runtime_key: &str,
    provider_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM llm_provider_configs WHERE runtime_key = ?1 AND provider_id = ?2",
        params![runtime_key, provider_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip_and_reload_per_runtime() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let cfg = StoredConfig {
            provider_id: "main".into(),
            api_type: "anthropic".into(),
            base_url: "https://x".into(),
            header_names: vec!["A".into()],
        };
        upsert(&conn, "claude-code", &cfg).unwrap();
        upsert(&conn, "codex-cli", &cfg).unwrap();
        assert_eq!(read_all(&conn).unwrap()["claude-code"], vec![cfg.clone()]);
        assert_eq!(read_runtime(&conn, "codex-cli").unwrap(), vec![cfg.clone()]);
        assert_eq!(read_one(&conn, "codex-cli", "main").unwrap(), Some(cfg));
        delete(&conn, "codex-cli", "main").unwrap();
        assert!(read_runtime(&conn, "codex-cli").unwrap().is_empty());
        assert!(read_one(&conn, "codex-cli", "main").unwrap().is_none());
    }
}
