use crate::db::{get_state, with_db};
use crate::error::AppError;
use crate::now_ms;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleSkill {
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) description: String,
    pub(crate) enabled: bool,
    pub(crate) ord: i64,
}

fn skill_from_row(row: &rusqlite::Row) -> rusqlite::Result<AppSkill> {
    Ok(AppSkill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(crate) fn list_skills_internal(state: &AppState) -> Result<Vec<AppSkill>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, content, created_at, updated_at
                 FROM app_skills ORDER BY updated_at DESC",
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let rows = stmt
            .query_map([], skill_from_row)
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::db(e.to_string()).to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub(crate) fn list_app_skills(state: State<'_, AppState>) -> Result<Vec<AppSkill>, String> {
    list_skills_internal(get_state(&state))
}

pub(crate) fn upsert_skill_internal(
    state: &AppState,
    input: AppSkillUpsert,
) -> Result<AppSkill, String> {
    let now = now_ms();
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("skill name cannot be empty").to_string());
    }
    if name.chars().any(|c| c.is_whitespace()) {
        return Err(AppError::validation("skill name cannot contain spaces").to_string());
    }
    let input_id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    with_db(state, |conn| {
        let existing_id_by_name: Option<String> = conn
            .query_row(
                "SELECT id FROM app_skills WHERE lower(name) = lower(?1)",
                params![&name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        if let Some(edit_id) = input_id {
            let created_at: i64 = conn
                .query_row(
                    "SELECT created_at FROM app_skills WHERE id = ?1",
                    params![&edit_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| AppError::db(e.to_string()).to_string())?
                .ok_or_else(|| {
                    AppError::not_found(format!("skill not found: {}", edit_id)).to_string()
                })?;
            if let Some(existing_id) = existing_id_by_name {
                if existing_id != edit_id {
                    return Err(AppError::already_exists(format!(
                        "skill name already exists: {}",
                        name
                    ))
                    .to_string());
                }
            }
            conn.execute(
                "UPDATE app_skills SET name = ?1, description = ?2, content = ?3, updated_at = ?4 WHERE id = ?5",
                params![&name, &input.description, &input.content, now, &edit_id],
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
            return Ok(AppSkill {
                id: edit_id,
                name,
                description: input.description,
                content: input.content,
                created_at,
                updated_at: now,
            });
        }
        if existing_id_by_name.is_some() {
            return Err(
                AppError::already_exists(format!("skill name already exists: {}", name))
                    .to_string(),
            );
        }
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO app_skills (id, name, description, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&id, &name, &input.description, &input.content, now, now,],
        )
        .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(AppSkill {
            id,
            name,
            description: input.description,
            content: input.content,
            created_at: now,
            updated_at: now,
        })
    })
}

pub(crate) fn delete_skill_internal(state: &AppState, id: &str) -> Result<(), String> {
    with_db(state, |conn| {
        conn.execute("DELETE FROM app_skills WHERE id = ?1", params![id])
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn upsert_app_skill(
    state: State<'_, AppState>,
    input: AppSkillUpsert,
) -> Result<AppSkill, String> {
    upsert_skill_internal(get_state(&state), input)
}

#[tauri::command]
pub(crate) fn delete_app_skill(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_skill_internal(get_state(&state), &id)
}

pub(crate) fn load_skill_by_name(state: &AppState, name: &str) -> Result<Option<AppSkill>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, name, description, content, created_at, updated_at
             FROM app_skills WHERE lower(name) = lower(?1)",
            params![name],
            skill_from_row,
        )
        .optional()
        .map_err(|e| AppError::db(e.to_string()).to_string())
    })
}

pub(crate) fn load_skills_by_names(state: &AppState, names: &[String]) -> Vec<AppSkill> {
    if names.is_empty() {
        return Vec::new();
    }
    with_db(state, |conn| {
        let placeholders = names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, name, description, content, created_at, updated_at
             FROM app_skills WHERE name IN ({placeholders})"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(names.iter()), skill_from_row)
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(s) = row {
                out.push(s);
            }
        }
        Ok(out)
    })
    .unwrap_or_default()
}

pub(crate) fn get_enabled_skills_for_role(
    state: &AppState,
    role_name: &str,
) -> Result<Vec<(String, String)>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.content
                 FROM role_skills rs
                 JOIN app_skills s ON s.id = rs.skill_id
                 WHERE rs.role_name = ?1 AND rs.enabled = 1
                 ORDER BY rs.ord ASC, s.name ASC",
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::db(e.to_string()).to_string())?);
        }
        Ok(out)
    })
}

pub(crate) fn list_all_skills_for_role_internal(
    state: &AppState,
    role_name: &str,
) -> Result<Vec<RoleSkill>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.content, s.description, COALESCE(rs.enabled, 0), COALESCE(rs.ord, 0)
                 FROM app_skills s
                 LEFT JOIN role_skills rs ON rs.skill_id = s.id AND rs.role_name = ?1
                 ORDER BY COALESCE(rs.ord, 999) ASC, s.name ASC",
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let rows = stmt
            .query_map(params![role_name], |row| {
                Ok(RoleSkill {
                    skill_id: row.get(0)?,
                    name: row.get(1)?,
                    content: row.get(2)?,
                    description: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    ord: row.get(5)?,
                })
            })
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::db(e.to_string()).to_string())?);
        }
        Ok(out)
    })
}

pub(crate) fn set_role_skills_internal(
    state: &AppState,
    role_name: &str,
    skills: Vec<(String, bool, i64)>,
) -> Result<(), String> {
    with_db(state, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM role_skills WHERE role_name = ?1",
            params![role_name],
        )
        .map_err(|e| AppError::db(e.to_string()).to_string())?;
        for (skill_id, enabled, ord) in skills {
            tx.execute(
                "INSERT INTO role_skills (role_name, skill_id, enabled, ord)
                 VALUES (?1, ?2, ?3, ?4)",
                params![role_name, skill_id, if enabled { 1 } else { 0 }, ord],
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        }
        tx.commit()
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn list_all_skills_for_role_cmd(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<RoleSkill>, String> {
    list_all_skills_for_role_internal(get_state(&state), &role_name)
}

#[tauri::command]
pub(crate) fn set_role_skills_cmd(
    state: State<'_, AppState>,
    role_name: String,
    skills: Vec<(String, bool, i64)>,
) -> Result<(), String> {
    set_role_skills_internal(get_state(&state), &role_name, skills)
}
