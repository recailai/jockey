use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

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
            .query_map([], |row| {
                Ok(AppSkill {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
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
    with_db(get_state(&state), |conn| {
        let existing_id: Option<String> = conn
            .query_row(
                "SELECT id FROM app_skills WHERE name = ?1",
                params![&name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        conn.execute(
            "INSERT INTO app_skills (id, name, description, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET
               description = excluded.description,
               content = excluded.content,
               updated_at = excluded.updated_at",
            params![
                &id,
                &name,
                &input.description,
                &input.content,
                now,
                now,
            ],
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

pub(crate) fn load_skill_by_name(state: &AppState, name: &str) -> Option<AppSkill> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, name, description, content, created_at, updated_at
             FROM app_skills WHERE name = ?1",
            params![name],
            |row| {
                Ok(AppSkill {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
}
