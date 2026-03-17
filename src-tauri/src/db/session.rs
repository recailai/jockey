use crate::db::context::{list_shared_context_internal, set_shared_context_internal};
use crate::db::role::{load_role, load_role_runtime_kind};
use crate::db::workflow::load_workflow;
use crate::db::{get_state, load_team_workspace_path, parse_payload, with_db};
use crate::types::*;
use crate::{acp, now_ms};
use rusqlite::params;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

pub(crate) fn record_session_event(
    state: &AppState,
    session_id: &str,
    event_type: &str,
    role_name: Option<&str>,
    payload: serde_json::Value,
) -> Result<SessionEvent, String> {
    let now = now_ms();
    let payload_text = payload.to_string();
    let id = with_db(state, |conn| {
        conn.execute(
            "INSERT INTO session_events (session_id, event_type, role_name, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, event_type, role_name, payload_text, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    })?;
    Ok(SessionEvent {
        id,
        session_id: session_id.to_string(),
        event_type: event_type.to_string(),
        role_name: role_name.map(|s| s.to_string()),
        payload,
        created_at: now,
    })
}

pub(crate) fn update_session_status(
    state: &AppState,
    session_id: &str,
    status: &str,
) -> Result<(), String> {
    let now = now_ms();
    with_db(state, |conn| {
        conn.execute(
            "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, session_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn list_sessions(
    state: State<'_, AppState>,
    team_id: String,
) -> Result<Vec<Session>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, team_id, workflow_id, status, initial_prompt, created_at, updated_at
                 FROM sessions WHERE team_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![team_id], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    status: row.get(3)?,
                    initial_prompt: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| e.to_string())?);
        }
        Ok(sessions)
    })
}

#[tauri::command]
pub(crate) fn list_session_events(
    state: State<'_, AppState>,
    session_id: String,
    cursor: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<SessionEvent>, String> {
    let bounded = limit.unwrap_or(200).min(1000);
    with_db(get_state(&state), |conn| {
        let mut events = Vec::new();
        if let Some(c) = cursor {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, event_type, role_name, payload, created_at
                     FROM session_events
                     WHERE session_id = ?1 AND id > ?2
                     ORDER BY id ASC
                     LIMIT ?3",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![&session_id, c, bounded])
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let payload_text: String = row.get(4).map_err(|e| e.to_string())?;
                events.push(SessionEvent {
                    id: row.get(0).map_err(|e| e.to_string())?,
                    session_id: row.get(1).map_err(|e| e.to_string())?,
                    event_type: row.get(2).map_err(|e| e.to_string())?,
                    role_name: row.get(3).map_err(|e| e.to_string())?,
                    payload: parse_payload(payload_text),
                    created_at: row.get(5).map_err(|e| e.to_string())?,
                });
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, event_type, role_name, payload, created_at
                     FROM session_events
                     WHERE session_id = ?1
                     ORDER BY id ASC
                     LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(params![&session_id, bounded])
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let payload_text: String = row.get(4).map_err(|e| e.to_string())?;
                events.push(SessionEvent {
                    id: row.get(0).map_err(|e| e.to_string())?,
                    session_id: row.get(1).map_err(|e| e.to_string())?,
                    event_type: row.get(2).map_err(|e| e.to_string())?,
                    role_name: row.get(3).map_err(|e| e.to_string())?,
                    payload: parse_payload(payload_text),
                    created_at: row.get(5).map_err(|e| e.to_string())?,
                })
            }
        }
        Ok(events)
    })
}

pub(crate) fn summarize_text(input: &str) -> String {
    let normalized = input.replace('\n', " ");
    let trimmed = normalized.trim();
    if trimmed.chars().count() <= 180 {
        return trimmed.to_string();
    }
    let mut out = String::new();
    for c in trimmed.chars().take(180) {
        out.push(c);
    }
    out
}

pub(crate) fn chunk_text(input: &str, size: usize) -> Vec<String> {
    if input.is_empty() {
        return vec![];
    }
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for ch in input.chars() {
        buf.push(ch);
        if buf.chars().count() >= size {
            chunks.push(buf.clone());
            buf.clear();
        }
    }
    if !buf.is_empty() {
        chunks.push(buf);
    }
    chunks
}

pub(crate) async fn run_workflow(
    app: AppHandle,
    session: Session,
    workflow: Workflow,
    seed_prompt: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let workspace_path = load_team_workspace_path(state.inner(), &session.team_id)
        .unwrap_or_else(|_| ".".to_string());
    let mut prompt = seed_prompt;
    for role_name in workflow.steps.iter() {
        let runtime_kind = load_role_runtime_kind(state.inner(), &session.team_id, role_name)
            .unwrap_or_else(|_| "mock".to_string());
        let started = WorkflowStateEvent {
            session_id: session.id.clone(),
            team_id: session.team_id.clone(),
            workflow_id: session.workflow_id.clone(),
            status: "running".to_string(),
            active_role: Some(role_name.clone()),
            message: format!("{role_name} started"),
            created_at: now_ms(),
        };
        app.emit("workflow/state_changed", started.clone())
            .map_err(|e| e.to_string())?;
        record_session_event(
            state.inner(),
            &session.id,
            "StepStarted",
            Some(role_name),
            json!({ "message": started.message, "runtimeKind": runtime_kind }),
        )?;

        let context_entries = list_shared_context_internal(state.inner(), &session.team_id)?;
        let context_pairs = context_entries
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect::<Vec<_>>();
        let role_data = load_role(state.inner(), &session.team_id, role_name).unwrap_or(None);
        let auto_approve = role_data.as_ref().map(|r| r.auto_approve).unwrap_or(true);
        let role_mode = role_data.as_ref().and_then(|r| r.mode.clone());
        let role_config: Vec<(String, String)> = role_data
            .as_ref()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.config_options_json).ok())
            .and_then(|v| {
                v.as_object().map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
            })
            .unwrap_or_default();
        let acp_result = acp::execute_runtime(
            &runtime_kind,
            role_name,
            &prompt,
            &context_pairs,
            &workspace_path,
            &app,
            auto_approve,
            vec![],
            role_mode,
            role_config,
        )
        .await;
        record_session_event(
            state.inner(),
            &session.id,
            "AdapterResult",
            Some(role_name),
            acp_result.meta.clone(),
        )?;

        let output = acp_result.output;
        let chunks = if acp_result.deltas.is_empty() {
            chunk_text(&output, 30)
        } else {
            acp_result.deltas
        };

        for chunk in chunks {
            let update = SessionUpdateEvent {
                session_id: session.id.clone(),
                team_id: session.team_id.clone(),
                workflow_id: session.workflow_id.clone(),
                role_name: role_name.clone(),
                delta: chunk.clone(),
                state: "writing".to_string(),
                done: false,
                created_at: now_ms(),
            };
            app.emit("session/update", update.clone())
                .map_err(|e| e.to_string())?;
            record_session_event(
                state.inner(),
                &session.id,
                "DeltaReceived",
                Some(role_name),
                json!({ "delta": chunk }),
            )?;
        }

        let summary = summarize_text(&output);
        set_shared_context_internal(
            state.inner(),
            &session.team_id,
            &format!("summary.{role_name}"),
            &summary,
        )?;
        record_session_event(
            state.inner(),
            &session.id,
            "StepCompleted",
            Some(role_name),
            json!({ "summary": summary }),
        )?;

        prompt = format!("{}\n\n{} handoff summary: {}", prompt, role_name, summary);
        let done_update = SessionUpdateEvent {
            session_id: session.id.clone(),
            team_id: session.team_id.clone(),
            workflow_id: session.workflow_id.clone(),
            role_name: role_name.clone(),
            delta: "".to_string(),
            state: "idle".to_string(),
            done: true,
            created_at: now_ms(),
        };
        app.emit("session/update", done_update)
            .map_err(|e| e.to_string())?;
    }

    update_session_status(state.inner(), &session.id, "completed")?;
    let completed = WorkflowStateEvent {
        session_id: session.id.clone(),
        team_id: session.team_id.clone(),
        workflow_id: session.workflow_id.clone(),
        status: "completed".to_string(),
        active_role: workflow.steps.last().cloned(),
        message: "workflow completed".to_string(),
        created_at: now_ms(),
    };
    app.emit("workflow/state_changed", completed.clone())
        .map_err(|e| e.to_string())?;
    record_session_event(
        state.inner(),
        &session.id,
        "WorkflowCompleted",
        workflow.steps.last().map(String::as_str),
        json!({ "message": completed.message }),
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn start_workflow(
    app: AppHandle,
    state: State<'_, AppState>,
    input: StartWorkflowInput,
) -> Result<Session, String> {
    let workflow = load_workflow(get_state(&state), &input.workflow_id)?;
    if workflow.team_id != input.team_id {
        return Err("workflow does not belong to team".to_string());
    }
    if workflow.steps.is_empty() {
        return Err("workflow has no steps".to_string());
    }

    let session = Session {
        id: Uuid::new_v4().to_string(),
        team_id: input.team_id,
        workflow_id: input.workflow_id,
        status: "running".to_string(),
        initial_prompt: input.initial_prompt.clone(),
        created_at: now_ms(),
        updated_at: now_ms(),
    };

    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO sessions (id, team_id, workflow_id, status, initial_prompt, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &session.id,
                &session.team_id,
                &session.workflow_id,
                &session.status,
                &session.initial_prompt,
                session.created_at,
                session.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;

    record_session_event(
        get_state(&state),
        &session.id,
        "WorkflowStarted",
        None,
        json!({ "prompt": session.initial_prompt }),
    )?;

    let workflow_notice = WorkflowStateEvent {
        session_id: session.id.clone(),
        team_id: session.team_id.clone(),
        workflow_id: session.workflow_id.clone(),
        status: "running".to_string(),
        active_role: workflow.steps.first().cloned(),
        message: "workflow started".to_string(),
        created_at: now_ms(),
    };
    app.emit("workflow/state_changed", workflow_notice)
        .map_err(|e| e.to_string())?;

    let app_handle = app.clone();
    let workflow_copy = workflow.clone();
    let session_copy = session.clone();
    let prompt_copy = input.initial_prompt;

    tauri::async_runtime::spawn(async move {
        let result = run_workflow(
            app_handle.clone(),
            session_copy.clone(),
            workflow_copy,
            prompt_copy,
        )
        .await;
        if let Err(error) = result {
            let state = app_handle.state::<AppState>();
            let _ = update_session_status(state.inner(), &session_copy.id, "error");
            let failed = WorkflowStateEvent {
                session_id: session_copy.id.clone(),
                team_id: session_copy.team_id.clone(),
                workflow_id: session_copy.workflow_id.clone(),
                status: "error".to_string(),
                active_role: None,
                message: error.clone(),
                created_at: now_ms(),
            };
            let _ = app_handle.emit("workflow/state_changed", failed.clone());
            let _ = record_session_event(
                state.inner(),
                &session_copy.id,
                "WorkflowFailed",
                None,
                json!({ "error": failed.message }),
            );
        }
    });

    Ok(session)
}
