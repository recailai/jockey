use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::*;
use rusqlite::params;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub(crate) fn create_workflow(
    state: State<'_, AppState>,
    name: String,
    steps: Vec<String>,
) -> Result<Workflow, String> {
    if steps.is_empty() {
        return Err("workflow steps cannot be empty".to_string());
    }
    let now = now_ms();
    let workflow = Workflow {
        id: Uuid::new_v4().to_string(),
        name,
        steps,
        created_at: now,
        updated_at: now,
    };
    let steps_json = serde_json::to_string(&workflow.steps).map_err(|e| e.to_string())?;

    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO workflows (id, name, steps_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &workflow.id,
                &workflow.name,
                &steps_json,
                workflow.created_at,
                workflow.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;

    Ok(workflow)
}

#[tauri::command]
pub(crate) fn list_workflows(
    state: State<'_, AppState>,
) -> Result<Vec<Workflow>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, steps_json, created_at, updated_at
                 FROM workflows ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let steps_json: String = row.get(2)?;
                let steps = serde_json::from_str::<Vec<String>>(&steps_json).unwrap_or_default();
                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    steps,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut workflows = Vec::new();
        for row in rows {
            workflows.push(row.map_err(|e| e.to_string())?);
        }
        Ok(workflows)
    })
}

pub(crate) fn load_workflow(state: &AppState, workflow_id: &str) -> Result<Workflow, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, name, steps_json, created_at, updated_at FROM workflows WHERE id = ?1",
            params![workflow_id],
            |row| {
                let steps_json: String = row.get(2)?;
                let steps = serde_json::from_str::<Vec<String>>(&steps_json).unwrap_or_default();
                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    steps,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    })
}
