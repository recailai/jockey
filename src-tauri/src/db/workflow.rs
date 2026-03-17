use crate::assistant::normalize_runtime_key;
use crate::db::role::upsert_role;
use crate::db::{get_state, with_db};
use crate::now_ms;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub(crate) fn create_workflow(
    state: State<'_, AppState>,
    team_id: String,
    name: String,
    steps: Vec<String>,
) -> Result<Workflow, String> {
    if steps.is_empty() {
        return Err("workflow steps cannot be empty".to_string());
    }
    let now = now_ms();
    let workflow = Workflow {
        id: Uuid::new_v4().to_string(),
        team_id,
        name,
        steps,
        created_at: now,
        updated_at: now,
    };
    let steps_json = serde_json::to_string(&workflow.steps).map_err(|e| e.to_string())?;

    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO workflows (id, team_id, name, steps_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &workflow.id,
                &workflow.team_id,
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
    team_id: String,
) -> Result<Vec<Workflow>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, team_id, name, steps_json, created_at, updated_at
                 FROM workflows WHERE team_id = ?1 ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![team_id], |row| {
                let steps_json: String = row.get(3)?;
                let steps = serde_json::from_str::<Vec<String>>(&steps_json).unwrap_or_default();
                Ok(Workflow {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    name: row.get(2)?,
                    steps,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
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
            "SELECT id, team_id, name, steps_json, created_at, updated_at FROM workflows WHERE id = ?1",
            params![workflow_id],
            |row| {
                let steps_json: String = row.get(3)?;
                let steps = serde_json::from_str::<Vec<String>>(&steps_json).unwrap_or_default();
                Ok(Workflow {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    name: row.get(2)?,
                    steps,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn resolve_workflow_id(
    state: State<'_, AppState>,
    team_id: &str,
    workflow_ref: &str,
) -> Result<String, String> {
    let by_id = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM workflows WHERE team_id = ?1 AND id = ?2",
            params![team_id, workflow_ref],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    if let Some(id) = by_id {
        return Ok(id);
    }
    let by_name = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM workflows WHERE team_id = ?1 AND name = ?2 ORDER BY updated_at DESC LIMIT 1",
            params![team_id, workflow_ref],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    by_name.ok_or_else(|| "workflow not found".to_string())
}

pub(crate) fn ensure_quick_workflow(
    state: State<'_, AppState>,
    team_id: &str,
    runtime_hint: &str,
) -> Result<Workflow, String> {
    let runtime = normalize_runtime_key(runtime_hint)
        .unwrap_or("mock")
        .to_string();
    upsert_role(
        state.clone(),
        team_id.to_string(),
        "Planner".to_string(),
        runtime.clone(),
        "Break down the task into a short, actionable plan with clear steps.".to_string(),
        None,
        None,
        None,
        None,
        None,
    )?;
    upsert_role(
        state.clone(),
        team_id.to_string(),
        "Executor".to_string(),
        runtime,
        "Execute the approved plan and provide concise progress and results.".to_string(),
        None,
        None,
        None,
        None,
        None,
    )?;
    create_workflow(
        state,
        team_id.to_string(),
        "quick".to_string(),
        vec!["Planner".to_string(), "Executor".to_string()],
    )
}

pub(crate) fn latest_workflow_id(
    state: State<'_, AppState>,
    team_id: &str,
) -> Result<Option<String>, String> {
    with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM workflows WHERE team_id = ?1 ORDER BY updated_at DESC LIMIT 1",
            params![team_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}
