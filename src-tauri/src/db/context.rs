use crate::db::with_db;
use crate::now_ms;
use crate::types::*;
use rusqlite::params;

pub(crate) fn set_shared_context_internal(
    state: &AppState,
    scope: &str,
    key: &str,
    value: &str,
) -> Result<ContextEntry, String> {
    let now = now_ms();
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO shared_context_snapshots (scope, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![scope, key, value, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(ContextEntry {
        scope: scope.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        updated_at: now,
    })
}

pub(crate) fn clear_shared_context_internal(
    state: &AppState,
    scope: &str,
    key: &str,
) -> Result<(), String> {
    with_db(state, |conn| {
        conn.execute(
            "DELETE FROM shared_context_snapshots WHERE scope = ?1 AND key = ?2",
            params![scope, key],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(())
}

pub(crate) fn list_shared_context_internal(
    state: &AppState,
    scope: &str,
) -> Result<Vec<ContextEntry>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT scope, key, value, updated_at
                 FROM shared_context_snapshots
                 WHERE scope = ?1
                 ORDER BY updated_at DESC, key ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![scope], |row| {
                Ok(ContextEntry {
                    scope: row.get(0)?,
                    key: row.get(1)?,
                    value: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut items = Vec::new();
        for item in rows {
            items.push(item.map_err(|e| e.to_string())?);
        }
        Ok(items)
    })
}

pub(crate) fn sanitize_dynamic_item_name(raw: &str) -> Option<String> {
    let clean = raw.trim().to_ascii_lowercase();
    if clean.is_empty() {
        return None;
    }
    if clean
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return Some(clean);
    }
    None
}

/// Catalog entries are scoped by runtime. `runtime_key` is empty for kinds that are
/// genuinely global (mcp, skill) and for legacy model rows that could not be attributed
/// to a runtime during migration.
pub(crate) fn upsert_dynamic_catalog_item(
    state: &AppState,
    kind: &str,
    runtime_key: &str,
    name: &str,
) -> Result<String, String> {
    let normalized = sanitize_dynamic_item_name(name)
        .ok_or_else(|| format!("invalid {} name: {}", kind, name))?;
    let now = now_ms();
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO dynamic_catalog_entries (kind, runtime_key, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(kind, runtime_key, name) DO UPDATE SET updated_at = excluded.updated_at",
            params![kind, runtime_key, &normalized, now, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(normalized)
}
