use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

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

#[tauri::command]
pub(crate) fn list_app_skills(state: State<'_, AppState>) -> Result<Vec<AppSkill>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, content, created_at, updated_at
                 FROM app_skills ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], skill_from_row)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub(crate) fn upsert_app_skill(
    state: State<'_, AppState>,
    input: AppSkillUpsert,
) -> Result<AppSkill, String> {
    let now = now_ms();
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("skill name cannot be empty".to_string());
    }
    if name.chars().any(|c| c.is_whitespace()) {
        return Err("skill name cannot contain spaces".to_string());
    }
    let input_id = input.id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    with_db(get_state(&state), |conn| {
        let existing_id_by_name: Option<String> = conn
            .query_row(
                "SELECT id FROM app_skills WHERE lower(name) = lower(?1)",
                params![&name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(edit_id) = input_id {
            let created_at: i64 = conn
                .query_row(
                    "SELECT created_at FROM app_skills WHERE id = ?1",
                    params![&edit_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("skill not found: {}", edit_id))?;
            if let Some(existing_id) = existing_id_by_name {
                if existing_id != edit_id {
                    return Err(format!("skill name already exists: {}", name));
                }
            }
            conn.execute(
                "UPDATE app_skills SET name = ?1, description = ?2, content = ?3, updated_at = ?4 WHERE id = ?5",
                params![&name, &input.description, &input.content, now, &edit_id],
            )
            .map_err(|e| e.to_string())?;
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
            return Err(format!("skill name already exists: {}", name));
        }
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO app_skills (id, name, description, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&id, &name, &input.description, &input.content, now, now,],
        )
        .map_err(|e| e.to_string())?;
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

#[tauri::command]
pub(crate) fn delete_app_skill(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_db(get_state(&state), |conn| {
        conn.execute("DELETE FROM app_skills WHERE id = ?1", params![&id])
            .map_err(|e| e.to_string())?;
        Ok(())
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
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(names.iter()), skill_from_row)
            .map_err(|e| e.to_string())?;
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
