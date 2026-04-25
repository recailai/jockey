use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleRule {
    pub rule_id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub ord: i64,
}

pub(crate) fn list_rules(state: &AppState) -> Result<Vec<Rule>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, content, description, created_at, updated_at FROM rules ORDER BY name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Rule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    content: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    })
}

pub(crate) fn upsert_rule(
    state: &AppState,
    id: &str,
    name: &str,
    content: &str,
    description: Option<&str>,
) -> Result<(), String> {
    if name.is_empty() {
        return Err("Rule name required".to_string());
    }
    let now = now_ms();
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO rules (id, name, content, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               content = excluded.content,
               description = excluded.description,
               updated_at = excluded.updated_at",
            params![id, name, content, description, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn delete_rule(state: &AppState, id: &str) -> Result<(), String> {
    with_db(state, |conn| {
        let deleted = conn
            .execute("DELETE FROM rules WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        if deleted == 0 {
            return Err(format!("Rule '{id}' not found"));
        }
        Ok(())
    })
}

pub(crate) fn set_role_rules(
    state: &AppState,
    role_name: &str,
    rules: &[(String, bool, i64)],
) -> Result<(), String> {
    with_db(state, |conn| {
        conn.execute(
            "DELETE FROM role_rules WHERE role_name = ?1",
            params![role_name],
        )
        .map_err(|e| e.to_string())?;
        for (rule_id, enabled, ord) in rules {
            conn.execute(
                "INSERT INTO role_rules (role_name, rule_id, enabled, ord) VALUES (?1, ?2, ?3, ?4)",
                params![role_name, rule_id, *enabled as i64, ord],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

pub(crate) fn list_role_rules(state: &AppState, role_name: &str) -> Result<Vec<RoleRule>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT rr.rule_id, r.name, r.content, r.description, rr.enabled, rr.ord
                 FROM role_rules rr
                 JOIN rules r ON r.id = rr.rule_id
                 WHERE rr.role_name = ?1
                 ORDER BY rr.ord ASC, r.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| {
                Ok(RoleRule {
                    rule_id: row.get(0)?,
                    name: row.get(1)?,
                    content: row.get(2)?,
                    description: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    ord: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    })
}

pub(crate) fn get_enabled_rules_for_role(
    state: &AppState,
    role_name: &str,
) -> Result<Vec<(String, String)>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT r.name, r.content
                 FROM role_rules rr
                 JOIN rules r ON r.id = rr.rule_id
                 WHERE rr.role_name = ?1 AND rr.enabled = 1
                 ORDER BY rr.ord ASC, r.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    })
}

#[tauri::command]
pub(crate) fn list_rules_cmd(state: State<'_, AppState>) -> Result<Vec<Rule>, String> {
    list_rules(get_state(&state))
}

#[tauri::command]
pub(crate) fn upsert_rule_cmd(
    state: State<'_, AppState>,
    id: String,
    name: String,
    content: String,
    description: Option<String>,
) -> Result<(), String> {
    upsert_rule(get_state(&state), &id, &name, &content, description.as_deref())
}

#[tauri::command]
pub(crate) fn delete_rule_cmd(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_rule(get_state(&state), &id)
}

#[tauri::command]
pub(crate) fn set_role_rules_cmd(
    state: State<'_, AppState>,
    role_name: String,
    rules: Vec<(String, bool, i64)>,
) -> Result<(), String> {
    set_role_rules(get_state(&state), &role_name, &rules)
}

#[tauri::command]
pub(crate) fn list_role_rules_cmd(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<RoleRule>, String> {
    list_role_rules(get_state(&state), &role_name)
}
