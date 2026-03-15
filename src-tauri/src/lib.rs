mod acp;

use dashmap::DashMap;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

struct AppState {
    db: Mutex<Connection>,
    shared_context: DashMap<String, String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Team {
    id: String,
    name: String,
    workspace_path: String,
    created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Role {
    id: String,
    team_id: String,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Workflow {
    id: String,
    team_id: String,
    name: String,
    steps: Vec<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    team_id: String,
    workflow_id: String,
    status: String,
    initial_prompt: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SessionEvent {
    id: i64,
    session_id: String,
    event_type: String,
    role_name: Option<String>,
    payload: Value,
    created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ContextEntry {
    team_id: String,
    key: String,
    value: String,
    updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SessionUpdateEvent {
    session_id: String,
    team_id: String,
    workflow_id: String,
    role_name: String,
    delta: String,
    state: String,
    done: bool,
    created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WorkflowStateEvent {
    session_id: String,
    team_id: String,
    workflow_id: String,
    status: String,
    active_role: Option<String>,
    message: String,
    created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ChatCommandResult {
    ok: bool,
    message: String,
    selected_team_id: Option<String>,
    selected_assistant: Option<String>,
    session_id: Option<String>,
    payload: Value,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AssistantRuntime {
    key: String,
    label: String,
    binary: String,
    available: bool,
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantChatInput {
    input: String,
    selected_team_id: Option<String>,
    selected_assistant: Option<String>,
    system_prompt: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssistantChatResponse {
    ok: bool,
    reply: String,
    selected_team_id: Option<String>,
    selected_assistant: Option<String>,
    session_id: Option<String>,
    command_result: Option<ChatCommandResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartWorkflowInput {
    team_id: String,
    workflow_id: String,
    initial_prompt: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn default_chat_cwd() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/tmp".to_string())
        })
}

fn abs_cwd(path: &str) -> String {
    let p = std::path::Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::PathBuf::from(default_chat_cwd()).join(p)
    };
    abs.canonicalize().unwrap_or(abs).to_string_lossy().to_string()
}

fn resolve_chat_cwd(state: &AppState, team_id: Option<&str>) -> String {
    let raw = if let Some(team) = team_id {
        load_team_workspace_path(state, team).unwrap_or_else(|_| default_chat_cwd())
    } else {
        default_chat_cwd()
    };
    abs_cwd(&raw)
}

fn clip_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>()
}

fn chat_log(event: &str, payload: Value) {
    eprintln!("[unionai.chat] {} {} {}", now_ms(), event, payload);
}

fn detect_reply_signals(reply: &str) -> Vec<String> {
    let text = reply.to_ascii_lowercase();
    let mut signals = Vec::new();
    if text.contains("memory show")
        || text.contains("memory refresh")
        || text.contains("memory add")
        || text.contains("memory list")
    {
        signals.push("memory".to_string());
    }
    if text.contains("available_commands_update") {
        signals.push("available_commands_update".to_string());
    }
    if text.contains("acp") {
        signals.push("acp".to_string());
    }
    signals
}

fn extract_command_output(reply: &str) -> Option<String> {
    let trimmed = reply.trim();
    if trimmed.starts_with('/') {
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with('`') && trimmed.ends_with('`') {
        let inner = trimmed.trim_matches('`').trim();
        if inner.starts_with('/') {
            return Some(inner.to_string());
        }
    }
    for line in trimmed.lines() {
        let cleaned = line.trim().trim_matches('`').trim();
        if cleaned.starts_with('/') {
            return Some(cleaned.to_string());
        }
    }
    None
}

fn shared_key(team_id: &str, key: &str) -> String {
    format!("{team_id}:{key}")
}

fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS teams (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          workspace_path TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS roles (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          role_name TEXT NOT NULL,
          runtime_kind TEXT NOT NULL,
          system_prompt TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          UNIQUE(team_id, role_name)
        );
        CREATE TABLE IF NOT EXISTS workflows (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          name TEXT NOT NULL,
          steps_json TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY,
          team_id TEXT NOT NULL,
          workflow_id TEXT NOT NULL,
          status TEXT NOT NULL,
          initial_prompt TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          event_type TEXT NOT NULL,
          role_name TEXT,
          payload TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS shared_context_snapshots (
          team_id TEXT NOT NULL,
          key TEXT NOT NULL,
          value TEXT NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(team_id, key)
        );
        ",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn get_state<'a>(state: &'a State<'_, AppState>) -> &'a AppState {
    state.inner()
}

fn with_db<T>(state: &AppState, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    f(&conn)
}

fn parse_payload(payload: String) -> Value {
    serde_json::from_str::<Value>(&payload).unwrap_or(json!({ "text": payload }))
}

fn record_session_event(
    state: &AppState,
    session_id: &str,
    event_type: &str,
    role_name: Option<&str>,
    payload: Value,
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

fn update_session_status(state: &AppState, session_id: &str, status: &str) -> Result<(), String> {
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

fn set_shared_context_internal(state: &AppState, team_id: &str, key: &str, value: &str) -> Result<ContextEntry, String> {
    let now = now_ms();
    state
        .shared_context
        .insert(shared_key(team_id, key), value.to_string());
    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO shared_context_snapshots (team_id, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![team_id, key, value, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(ContextEntry {
        team_id: team_id.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        updated_at: now,
    })
}

fn load_workflow(state: &AppState, workflow_id: &str) -> Result<Workflow, String> {
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

fn load_team_workspace_path(state: &AppState, team_id: &str) -> Result<String, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT workspace_path FROM teams WHERE id = ?1",
            params![team_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
}

fn load_role_runtime_kind(state: &AppState, team_id: &str, role_name: &str) -> Result<String, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT runtime_kind FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|item| item.unwrap_or_else(|| "mock".to_string()))
    })
}

fn summarize_text(input: &str) -> String {
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

fn chunk_text(input: &str, size: usize) -> Vec<String> {
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

fn list_shared_context_internal(state: &AppState, team_id: &str) -> Result<Vec<ContextEntry>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT team_id, key, value, updated_at
                 FROM shared_context_snapshots
                 WHERE team_id = ?1
                 ORDER BY updated_at DESC, key ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![team_id], |row| {
                Ok(ContextEntry {
                    team_id: row.get(0)?,
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

async fn run_workflow(app: AppHandle, session: Session, workflow: Workflow, seed_prompt: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let workspace_path = load_team_workspace_path(state.inner(), &session.team_id)
        .unwrap_or_else(|_| ".".to_string());
    let mut prompt = seed_prompt;
    for role_name in workflow.steps.iter() {
        let runtime_kind =
            load_role_runtime_kind(state.inner(), &session.team_id, role_name).unwrap_or_else(|_| "mock".to_string());
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
        let acp_result = acp::execute_runtime(
            &runtime_kind,
            role_name,
            &prompt,
            &context_pairs,
            &workspace_path,
            &app,
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
fn create_team(state: State<'_, AppState>, name: String, workspace_path: String) -> Result<Team, String> {
    let team = Team {
        id: Uuid::new_v4().to_string(),
        name,
        workspace_path,
        created_at: now_ms(),
    };
    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO teams (id, name, workspace_path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![&team.id, &team.name, &team.workspace_path, team.created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(team)
}

#[tauri::command]
fn list_teams(state: State<'_, AppState>) -> Result<Vec<Team>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare("SELECT id, name, workspace_path, created_at FROM teams ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Team {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    workspace_path: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut teams = Vec::new();
        for row in rows {
            teams.push(row.map_err(|e| e.to_string())?);
        }
        Ok(teams)
    })
}

fn upsert_role(
    state: State<'_, AppState>,
    team_id: String,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
) -> Result<Role, String> {
    let now = now_ms();
    let existing_id = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![&team_id, &role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;

    let role = Role {
        id: existing_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id,
        role_name,
        runtime_kind,
        system_prompt,
        created_at: now,
        updated_at: now,
    };

    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO roles (id, team_id, role_name, runtime_kind, system_prompt, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(team_id, role_name) DO UPDATE SET
               runtime_kind = excluded.runtime_kind,
               system_prompt = excluded.system_prompt,
               updated_at = excluded.updated_at",
            params![
                &role.id,
                &role.team_id,
                &role.role_name,
                &role.runtime_kind,
                &role.system_prompt,
                role.created_at,
                role.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;

    Ok(role)
}

#[tauri::command]
async fn upsert_role_cmd(
    state: State<'_, AppState>,
    team_id: String,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
) -> Result<Role, String> {
    let role = upsert_role(state.clone(), team_id, role_name, runtime_kind, system_prompt)?;
    // Pre-warm the role's ACP session in background so first @Role message is instant.
    let runtime_for_warmup = role.runtime_kind.clone();
    let role_for_warmup = role.role_name.clone();
    let cwd_for_warmup = resolve_chat_cwd(get_state(&state), Some(&role.team_id));
    tauri::async_runtime::spawn(async move {
        acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &cwd_for_warmup).await;
    });
    Ok(role)
}

#[tauri::command]
fn list_roles(state: State<'_, AppState>, team_id: String) -> Result<Vec<Role>, String> {
    with_db(get_state(&state), |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, team_id, role_name, runtime_kind, system_prompt, created_at, updated_at
                 FROM roles WHERE team_id = ?1 ORDER BY role_name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![team_id], |row| {
                Ok(Role {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    role_name: row.get(2)?,
                    runtime_kind: row.get(3)?,
                    system_prompt: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut roles = Vec::new();
        for row in rows {
            roles.push(row.map_err(|e| e.to_string())?);
        }
        Ok(roles)
    })
}

#[tauri::command]
fn create_workflow(
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
fn list_workflows(state: State<'_, AppState>, team_id: String) -> Result<Vec<Workflow>, String> {
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

#[tauri::command]
fn list_sessions(state: State<'_, AppState>, team_id: String) -> Result<Vec<Session>, String> {
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
fn list_session_events(
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

#[tauri::command]
fn set_shared_context(
    state: State<'_, AppState>,
    team_id: String,
    key: String,
    value: String,
) -> Result<ContextEntry, String> {
    set_shared_context_internal(get_state(&state), &team_id, &key, &value)
}

#[tauri::command]
fn get_shared_context(
    state: State<'_, AppState>,
    team_id: String,
    key: String,
) -> Result<Option<ContextEntry>, String> {
    let cache_key = shared_key(&team_id, &key);
    if let Some(v) = get_state(&state).shared_context.get(&cache_key) {
        return Ok(Some(ContextEntry {
            team_id,
            key,
            value: v.value().clone(),
            updated_at: now_ms(),
        }));
    }

    let db_value = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT value, updated_at FROM shared_context_snapshots WHERE team_id = ?1 AND key = ?2",
            params![&team_id, &key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;

    if let Some((value, updated_at)) = db_value {
        get_state(&state)
            .shared_context
            .insert(cache_key, value.clone());
        return Ok(Some(ContextEntry {
            team_id,
            key,
            value,
            updated_at,
        }));
    }

    Ok(None)
}

#[tauri::command]
fn list_shared_context(state: State<'_, AppState>, team_id: String) -> Result<Vec<ContextEntry>, String> {
    list_shared_context_internal(get_state(&state), &team_id)
}

#[tauri::command]
async fn start_workflow(
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
        let result = run_workflow(app_handle.clone(), session_copy.clone(), workflow_copy, prompt_copy).await;
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

fn split_tokens(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

fn ensure_team_selected(selected_team_id: Option<String>) -> Result<String, String> {
    selected_team_id.ok_or_else(|| "no selected team".to_string())
}

fn resolve_team_id(state: State<'_, AppState>, value: &str) -> Result<String, String> {
    let by_id = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM teams WHERE id = ?1",
            params![value],
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
            "SELECT id FROM teams WHERE name = ?1 ORDER BY created_at DESC LIMIT 1",
            params![value],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    by_name.ok_or_else(|| "team not found".to_string())
}

fn latest_workflow_id(state: State<'_, AppState>, team_id: &str) -> Result<Option<String>, String> {
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

fn resolve_workflow_id(state: State<'_, AppState>, team_id: &str, workflow_ref: &str) -> Result<String, String> {
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

fn resolve_role_runtime(state: State<'_, AppState>, team_id: &str, role_name: &str) -> Result<String, String> {
    with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT runtime_kind FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|item| item.unwrap_or_else(|| "mock".to_string()))
    })
}

fn resolve_role_prompt(state: State<'_, AppState>, team_id: &str, role_name: &str) -> Result<String, String> {
    with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT system_prompt FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|item| item.unwrap_or_default())
    })
}

fn delete_team_cascade(state: State<'_, AppState>, team_id: &str) -> Result<(), String> {
    with_db(get_state(&state), |conn| {
        conn.execute("DELETE FROM session_events WHERE session_id IN (SELECT id FROM sessions WHERE team_id = ?1)", params![team_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM sessions WHERE team_id = ?1", params![team_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM workflows WHERE team_id = ?1", params![team_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM roles WHERE team_id = ?1", params![team_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM shared_context_snapshots WHERE team_id = ?1", params![team_id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM teams WHERE id = ?1", params![team_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn detect_binary_version(binary: &str) -> Option<String> {
    match Command::new(binary).arg("--version").output() {
        Ok(output) => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            text.lines().next().map(|s| s.trim().to_string())
        }
        Err(_) => None,
    }
}

static ASSISTANT_CATALOG: OnceLock<Vec<AssistantRuntime>> = OnceLock::new();

fn assistant_catalog() -> &'static Vec<AssistantRuntime> {
    ASSISTANT_CATALOG.get_or_init(|| build_assistant_catalog())
}

fn build_assistant_catalog() -> Vec<AssistantRuntime> {
    let (claude_ok, claude_bin) = acp::probe_runtime("claude-code")
        .unwrap_or((false, "claude-code adapter unavailable".to_string()));
    let (gemini_ok, gemini_bin) = acp::probe_runtime("gemini-cli")
        .unwrap_or((false, "gemini-cli adapter unavailable".to_string()));
    let (codex_ok, codex_bin) = acp::probe_runtime("codex-cli")
        .unwrap_or((false, "codex-cli adapter unavailable".to_string()));
    let claude_v = if claude_ok && claude_bin != "npx" {
        detect_binary_version(&claude_bin)
    } else {
        None
    };
    let gemini_v = if gemini_ok && gemini_bin != "npx" {
        detect_binary_version(&gemini_bin)
    } else {
        None
    };
    let codex_v = if codex_ok && codex_bin != "npx" {
        detect_binary_version(&codex_bin)
    } else {
        None
    };
    vec![
        AssistantRuntime {
            key: "claude-code".to_string(),
            label: "Claude Code".to_string(),
            binary: claude_bin.clone(),
            available: claude_ok,
            version: claude_v,
        },
        AssistantRuntime {
            key: "gemini-cli".to_string(),
            label: "Gemini CLI".to_string(),
            binary: gemini_bin.clone(),
            available: gemini_ok,
            version: gemini_v,
        },
        AssistantRuntime {
            key: "codex-cli".to_string(),
            label: "Codex CLI".to_string(),
            binary: codex_bin.clone(),
            available: codex_ok,
            version: codex_v,
        },
    ]
}

fn normalize_runtime_key(runtime: &str) -> Option<&'static str> {
    match runtime.trim().to_ascii_lowercase().as_str() {
        "gemini" | "gemini-cli" => Some("gemini-cli"),
        "claude" | "claude-code" | "claude-acp" => Some("claude-code"),
        "codex" | "codex-cli" | "codex-acp" => Some("codex-cli"),
        "mock" => Some("mock"),
        _ => None,
    }
}

fn build_unionai_tool_prompt() -> &'static str {
    "You are UnionAI assistant. Answer the user's question directly and concisely.\n\
IMPORTANT: Do NOT use any tools, read files, run commands, or explore the filesystem.\n\
\n\
To control UnionAI, output ONLY the slash command on its own line — UnionAI will execute it automatically:\n\
  /team list | /team create <name> | /team select <id> | /team delete <id>\n\
  /role list | /role bind <role> <runtime> [prompt] | /role delete <role>\n\
  /workflow list | /workflow create <name> <r1,r2> | /workflow start <name> <prompt>\n\
  /session list | /session stop <id>\n\
  /context list | /context set <key> <value> | /context get <key>\n\
  /init <team> <runtime> | /run <prompt>\n\
These are UI commands for UnionAI, NOT tools you can invoke. Only output them when the user explicitly asks to perform a UnionAI action."
}

fn infer_builtin_commands(input: &str, selected_assistant: Option<&str>) -> Option<Vec<String>> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.to_ascii_lowercase();
    if matches!(normalized.as_str(), "ls" | "list" | "team list" | "teams" | "list teams") {
        return Some(vec!["/team list".to_string()]);
    }
    if normalized.starts_with("ls ")
        || normalized == "ls一下"
        || normalized == "ls 一下"
        || normalized == "列出 team"
        || normalized == "列出teams"
        || normalized == "看看 team"
        || normalized == "看看teams"
    {
        return Some(vec!["/team list".to_string()]);
    }
    if normalized.contains("how many role")
        || normalized.contains("role count")
        || normalized.contains("how many roles")
        || normalized.contains("多少 role")
        || normalized.contains("多少 roles")
        || normalized.contains("几个 role")
    {
        return Some(vec!["/role list".to_string()]);
    }
    if matches!(
        normalized.as_str(),
        "new team" | "create team" | "newteam" | "创建team" | "创建团队"
    ) {
        return Some(vec![format!("/team create team-{}", now_ms() % 100000)]);
    }
    if let Some(name) = normalized.strip_prefix("new team ") {
        let clean = name.trim();
        if !clean.is_empty() {
            return Some(vec![format!("/team create {}", clean)]);
        }
    }
    if let Some(name) = normalized.strip_prefix("create team ") {
        let clean = name.trim();
        if !clean.is_empty() {
            return Some(vec![format!("/team create {}", clean)]);
        }
    }
    if let Some(name) = normalized.strip_prefix("select team ") {
        let clean = name.trim();
        if !clean.is_empty() {
            return Some(vec![format!("/team select {}", clean)]);
        }
    }
    if normalized.contains("new role") || normalized.contains("create role") {
        let runtime = selected_assistant.unwrap_or("gemini-cli");
        let mut commands = Vec::new();
        let team_name = if let Some(idx) = normalized.find(" for ") {
            let suffix = normalized[idx + 5..].trim();
            suffix.strip_suffix(" team").unwrap_or(suffix).trim()
        } else {
            ""
        };
        if !team_name.is_empty() {
            commands.push(format!("/team select {}", team_name));
        }
        let role_name = if let Some(rest) = normalized.strip_prefix("new role ") {
            let head = rest.split(" for ").next().unwrap_or("").trim();
            let tokens = head.split_whitespace().collect::<Vec<_>>();
            if tokens.is_empty() || tokens[0] == "for" || tokens[0] == "role" {
                "Developer".to_string()
            } else if tokens[0] == "as" && tokens.len() > 1 {
                tokens[1].to_string()
            } else {
                tokens[0].to_string()
            }
        } else if let Some(rest) = normalized.strip_prefix("create role ") {
            let head = rest.split(" for ").next().unwrap_or("").trim();
            let tokens = head.split_whitespace().collect::<Vec<_>>();
            if tokens.is_empty() || tokens[0] == "for" || tokens[0] == "role" {
                "Developer".to_string()
            } else if tokens[0] == "as" && tokens.len() > 1 {
                tokens[1].to_string()
            } else {
                tokens[0].to_string()
            }
        } else {
            "Developer".to_string()
        };
        commands.push(format!("/role bind {} {}", role_name, runtime));
        return Some(commands);
    }
    if normalized == "help" {
        return Some(vec!["/help".to_string()]);
    }
    None
}

#[tauri::command]
async fn apply_chat_command(
    app: AppHandle,
    state: State<'_, AppState>,
    input: String,
    selected_team_id: Option<String>,
    selected_assistant: Option<String>,
) -> Result<ChatCommandResult, String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Err("chat set command must start with /".to_string());
    }

    let tokens = split_tokens(trimmed);
    if tokens.is_empty() {
        return Err("empty command".to_string());
    }

    let mut result = ChatCommandResult {
        ok: true,
        message: "ok".to_string(),
        selected_team_id: selected_team_id.clone(),
        selected_assistant: selected_assistant.clone(),
        session_id: None,
        payload: json!({}),
    };

    match tokens.as_slice() {
        ["/help"] => {
            result.message = "command list".to_string();
            result.payload = json!({ "help": build_unionai_tool_prompt() });
        }
        ["/assistant", "list"] => {
            let assistants = assistant_catalog();
            result.message = "assistant list".to_string();
            result.payload = json!({ "assistants": assistants });
        }
        ["/assistant", "select", runtime] => {
            match normalize_runtime_key(runtime) {
                Some(normalized) => {
                    result.selected_assistant = Some(normalized.to_string());
                    result.message = format!("assistant selected: {}", normalized);
                    result.payload = json!({ "assistant": normalized });
                    let runtime_key = normalized.to_string();
                    let prewarm_cwd = resolve_chat_cwd(get_state(&state), selected_team_id.as_deref());
                    tauri::async_runtime::spawn(async move {
                        acp::prewarm(&runtime_key, &prewarm_cwd).await;
                    });
                }
                None => {
                    result.ok = false;
                    result.message = format!("unsupported assistant: {}", runtime);
                    result.payload = json!({ "assistant": runtime });
                }
            }
        }
        ["/init", team_name, runtime_kind] => {
            let normalized_runtime = normalize_runtime_key(runtime_kind)
                .map(|v| v.to_string())
                .unwrap_or_else(|| (*runtime_kind).to_string());
            let team = create_team(state.clone(), (*team_name).to_string(), ".".to_string())?;
            let roles = ["Architect", "Developer", "Reviewer"];
            for role in roles {
                let _ = upsert_role(
                    state.clone(),
                    team.id.clone(),
                    role.to_string(),
                    normalized_runtime.clone(),
                    "default-system-prompt".to_string(),
                )?;
            }
            let workflow = create_workflow(
                state.clone(),
                team.id.clone(),
                "default".to_string(),
                vec![
                    "Architect".to_string(),
                    "Developer".to_string(),
                    "Reviewer".to_string(),
                ],
            )?;
            result.message = format!("initialized team {} with workflow {}", team.name, workflow.name);
            result.selected_team_id = Some(team.id.clone());
            result.selected_assistant = Some(normalized_runtime.clone());
            result.payload = json!({ "team": team, "workflow": workflow });
        }
        ["/run", prompt @ ..] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let workflow_id = latest_workflow_id(state.clone(), &team_id)?
                .ok_or_else(|| "no workflow for selected team".to_string())?;
            let prompt_text = if prompt.is_empty() {
                "run".to_string()
            } else {
                prompt.join(" ")
            };
            let session = start_workflow(
                app.clone(),
                state.clone(),
                StartWorkflowInput {
                    team_id,
                    workflow_id,
                    initial_prompt: prompt_text,
                },
            )
            .await?;
            result.message = format!("run started: {}", session.id);
            result.session_id = Some(session.id.clone());
            result.payload = json!({ "session": session });
        }
        ["/team", "list"] => {
            let teams = list_teams(state.clone())?;
            result.message = format!("{} teams", teams.len());
            result.payload = json!({ "teams": teams });
        }
        ["/team", "create", name] => {
            let team = create_team(state.clone(), (*name).to_string(), ".".to_string())?;
            result.message = format!("team created: {}", team.name);
            result.selected_team_id = Some(team.id.clone());
            result.payload = json!({ "team": team });
        }
        ["/team", "update", team_ref, new_name] => {
            let team_id = resolve_team_id(state.clone(), team_ref)?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE teams SET name = ?1 WHERE id = ?2",
                    params![new_name, &team_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("team updated: {}", new_name);
            result.payload = json!({ "teamId": team_id, "name": new_name });
        }
        ["/team", "delete", team_ref] => {
            let team_id = resolve_team_id(state.clone(), team_ref)?;
            delete_team_cascade(state.clone(), &team_id)?;
            if selected_team_id == Some(team_id.clone()) {
                result.selected_team_id = None;
            }
            result.message = format!("team deleted: {}", team_id);
            result.payload = json!({ "teamId": team_id });
        }
        ["/team", "select", team_ref] => {
            let resolved = resolve_team_id(state.clone(), team_ref)?;
            result.message = format!("team selected: {resolved}");
            result.selected_team_id = Some(resolved.clone());
            result.payload = json!({ "teamId": resolved });
            let prewarm_cwd = resolve_chat_cwd(get_state(&state), result.selected_team_id.as_deref());
            if let Some(runtime_key) = result.selected_assistant.clone() {
                let runtime_for_warmup = runtime_key.clone();
                let cwd_for_warmup = prewarm_cwd.clone();
                tauri::async_runtime::spawn(async move {
                    acp::prewarm(&runtime_for_warmup, &cwd_for_warmup).await;
                });
            }
            if let Some(team_id) = result.selected_team_id.clone() {
                if let Ok(team_roles) = list_roles(state.clone(), team_id) {
                    for role in team_roles {
                        let runtime_for_warmup = role.runtime_kind.clone();
                        let role_for_warmup = role.role_name.clone();
                        let cwd_for_warmup = prewarm_cwd.clone();
                        tauri::async_runtime::spawn(async move {
                            acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &cwd_for_warmup).await;
                        });
                    }
                }
            }
        }
        ["/team", team_ref] => {
            let resolved = resolve_team_id(state.clone(), team_ref)?;
            result.message = format!("team selected: {resolved}");
            result.selected_team_id = Some(resolved.clone());
            result.payload = json!({ "teamId": resolved });
            let prewarm_cwd = resolve_chat_cwd(get_state(&state), result.selected_team_id.as_deref());
            if let Some(runtime_key) = result.selected_assistant.clone() {
                let runtime_for_warmup = runtime_key.clone();
                let cwd_for_warmup = prewarm_cwd.clone();
                tauri::async_runtime::spawn(async move {
                    acp::prewarm(&runtime_for_warmup, &cwd_for_warmup).await;
                });
            }
            if let Some(team_id) = result.selected_team_id.clone() {
                if let Ok(team_roles) = list_roles(state.clone(), team_id) {
                    for role in team_roles {
                        let runtime_for_warmup = role.runtime_kind.clone();
                        let role_for_warmup = role.role_name.clone();
                        let cwd_for_warmup = prewarm_cwd.clone();
                        tauri::async_runtime::spawn(async move {
                            acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &cwd_for_warmup).await;
                        });
                    }
                }
            }
        }
        ["/role", "list"] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let roles = list_roles(state.clone(), team_id)?;
            result.message = format!("{} roles", roles.len());
            result.payload = json!({ "roles": roles });
        }
        ["/role", "bind", role_name, runtime_kind, prompt @ ..] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let normalized_runtime = normalize_runtime_key(runtime_kind)
                .map(|v| v.to_string())
                .unwrap_or_else(|| (*runtime_kind).to_string());
            let role = upsert_role(
                state.clone(),
                team_id.clone(),
                (*role_name).to_string(),
                normalized_runtime.clone(),
                if prompt.is_empty() {
                    "default-system-prompt".to_string()
                } else {
                    prompt.join(" ")
                },
            )?;
            result.message = format!("role bound: {}", role.role_name);
            result.payload = json!({ "role": role });
            let runtime_for_warmup = normalized_runtime.clone();
            let role_for_warmup = (*role_name).to_string();
            let prewarm_cwd = resolve_chat_cwd(get_state(&state), Some(&team_id));
            tauri::async_runtime::spawn(async move {
                acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &prewarm_cwd).await;
            });
        }
        ["/role", "prompt", role_name, prompt @ ..] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let runtime = resolve_role_runtime(state.clone(), &team_id, role_name)?;
            let role = upsert_role(
                state.clone(),
                team_id,
                (*role_name).to_string(),
                runtime,
                prompt.join(" "),
            )?;
            result.message = format!("role prompt updated: {}", role.role_name);
            result.payload = json!({ "role": role });
        }
        ["/role", "delete", role_name] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM roles WHERE team_id = ?1 AND role_name = ?2",
                    params![&team_id, role_name],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("role deleted: {}", role_name);
            result.payload = json!({ "roleName": role_name });
        }
        ["/workflow", "list"] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let workflows = list_workflows(state.clone(), team_id)?;
            result.message = format!("{} workflows", workflows.len());
            result.payload = json!({ "workflows": workflows });
        }
        ["/workflow", "create", name, steps_csv] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let steps = steps_csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let wf = create_workflow(state.clone(), team_id, (*name).to_string(), steps)?;
            result.message = format!("workflow created: {}", wf.name);
            result.payload = json!({ "workflow": wf });
        }
        ["/workflow", "update", workflow_ref, steps_csv] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let workflow_id = resolve_workflow_id(state.clone(), &team_id, workflow_ref)?;
            let steps = steps_csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if steps.is_empty() {
                return Err("workflow steps cannot be empty".to_string());
            }
            let steps_json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "UPDATE workflows SET steps_json = ?1, updated_at = ?2 WHERE id = ?3",
                    params![steps_json, now_ms(), &workflow_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("workflow updated: {}", workflow_id);
            result.payload = json!({ "workflowId": workflow_id, "steps": steps });
        }
        ["/workflow", "delete", workflow_ref] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let workflow_id = resolve_workflow_id(state.clone(), &team_id, workflow_ref)?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM session_events WHERE session_id IN (SELECT id FROM sessions WHERE workflow_id = ?1)",
                    params![&workflow_id],
                )
                .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM sessions WHERE workflow_id = ?1", params![&workflow_id])
                    .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM workflows WHERE id = ?1", params![&workflow_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("workflow deleted: {}", workflow_id);
            result.payload = json!({ "workflowId": workflow_id });
        }
        ["/workflow", "start", workflow_ref, prompt @ ..] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let workflow_id = resolve_workflow_id(state.clone(), &team_id, workflow_ref)?;
            let prompt_text = if prompt.is_empty() {
                "start".to_string()
            } else {
                prompt.join(" ")
            };
            let session = start_workflow(
                app.clone(),
                state.clone(),
                StartWorkflowInput {
                    team_id,
                    workflow_id: workflow_id.clone(),
                    initial_prompt: prompt_text,
                },
            )
            .await?;
            result.message = format!("workflow started: {}", session.id);
            result.session_id = Some(session.id.clone());
            result.payload = json!({ "session": session });
        }
        ["/session", "list"] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let sessions = list_sessions(state.clone(), team_id)?;
            result.message = format!("{} sessions", sessions.len());
            result.payload = json!({ "sessions": sessions });
        }
        ["/session", "events", session_id] => {
            let events = list_session_events(state.clone(), (*session_id).to_string(), None, Some(300))?;
            result.message = format!("{} session events", events.len());
            result.payload = json!({ "events": events, "sessionId": session_id });
        }
        ["/session", "stop", session_id] => {
            update_session_status(get_state(&state), session_id, "stopped")?;
            result.message = format!("session stopped: {}", session_id);
            result.payload = json!({ "sessionId": session_id });
        }
        ["/session", "delete", session_id] => {
            with_db(get_state(&state), |conn| {
                conn.execute("DELETE FROM session_events WHERE session_id = ?1", params![session_id])
                    .map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            result.message = format!("session deleted: {}", session_id);
            result.payload = json!({ "sessionId": session_id });
        }
        ["/context", "list"] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let entries = list_shared_context(state.clone(), team_id)?;
            result.message = format!("{} context entries", entries.len());
            result.payload = json!({ "entries": entries });
        }
        ["/context", "set", key, value @ ..] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let text = value.join(" ");
            let entry = set_shared_context(state.clone(), team_id, (*key).to_string(), text)?;
            result.message = format!("context set: {}", entry.key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "get", key] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            let entry = get_shared_context(state.clone(), team_id, (*key).to_string())?;
            result.message = format!("context fetched: {}", key);
            result.payload = json!({ "entry": entry });
        }
        ["/context", "delete", key] => {
            let team_id = ensure_team_selected(selected_team_id.clone())?;
            with_db(get_state(&state), |conn| {
                conn.execute(
                    "DELETE FROM shared_context_snapshots WHERE team_id = ?1 AND key = ?2",
                    params![&team_id, key],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            get_state(&state).shared_context.remove(&shared_key(&team_id, key));
            result.message = format!("context deleted: {}", key);
            result.payload = json!({ "key": key });
        }
        _ => {
            result.ok = false;
            result.message = "unsupported command".to_string();
        }
    }

    app.emit("command/applied", result.clone())
        .map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
fn detect_assistants() -> Result<Vec<AssistantRuntime>, String> {
    Ok(assistant_catalog().clone())
}

#[tauri::command]
async fn assistant_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AssistantChatInput,
) -> Result<AssistantChatResponse, String> {
    let started = Instant::now();
    let text = input.input.trim().to_string();
    chat_log(
        "request.start",
        json!({
            "inputSize": text.len(),
            "preview": clip_text(&text, 120),
            "selectedTeamId": input.selected_team_id.clone(),
            "selectedAssistant": input.selected_assistant.clone()
        }),
    );
    if text.is_empty() {
        chat_log(
            "request.empty",
            json!({
                "latencyMs": started.elapsed().as_millis()
            }),
        );
        return Ok(AssistantChatResponse {
            ok: false,
            reply: "empty input".to_string(),
            selected_team_id: input.selected_team_id,
            selected_assistant: input.selected_assistant,
            session_id: None,
            command_result: None,
        });
    }

    if text.starts_with('/') {
        let route_started = Instant::now();
        let command_result = apply_chat_command(
            app,
            state,
            text,
            input.selected_team_id.clone(),
            input.selected_assistant.clone(),
        )
        .await?;
        chat_log(
            "route.command",
            json!({
                "latencyMs": route_started.elapsed().as_millis(),
                "totalLatencyMs": started.elapsed().as_millis(),
                "ok": command_result.ok,
                "message": clip_text(&command_result.message, 120)
            }),
        );
        return Ok(AssistantChatResponse {
            ok: command_result.ok,
            reply: command_result.message.clone(),
            selected_team_id: command_result.selected_team_id.clone(),
            selected_assistant: command_result.selected_assistant.clone(),
            session_id: command_result.session_id.clone(),
            command_result: Some(command_result),
        });
    }

    if let Some(commands) = infer_builtin_commands(&text, input.selected_assistant.as_deref()) {
        chat_log(
            "route.builtin.detected",
            json!({
                "count": commands.len(),
                "commands": commands.clone()
            }),
        );
        let mut current_team = input.selected_team_id.clone();
        let mut current_assistant = input.selected_assistant.clone();
        let mut last_result: Option<ChatCommandResult> = None;
        let mut logs = Vec::new();

        for cmd in commands {
            let cmd_started = Instant::now();
            let result = apply_chat_command(
                app.clone(),
                state.clone(),
                cmd.clone(),
                current_team.clone(),
                current_assistant.clone(),
            )
            .await?;
            chat_log(
                "route.builtin.command",
                json!({
                    "command": cmd,
                    "latencyMs": cmd_started.elapsed().as_millis(),
                    "ok": result.ok,
                    "message": clip_text(&result.message, 120)
                }),
            );
            current_team = result.selected_team_id.clone().or(current_team);
            current_assistant = result.selected_assistant.clone().or(current_assistant);
            logs.push(format!("{} => {}", cmd, result.message));
            last_result = Some(result);
        }

        if let Some(command_result) = last_result {
            chat_log(
                "route.builtin.done",
                json!({
                    "latencyMs": started.elapsed().as_millis(),
                    "ok": command_result.ok
                }),
            );
            return Ok(AssistantChatResponse {
                ok: command_result.ok,
                reply: logs.join("\n"),
                selected_team_id: current_team,
                selected_assistant: current_assistant,
                session_id: command_result.session_id.clone(),
                command_result: Some(command_result),
            });
        }
    }

    let assistant = input
        .selected_assistant
        .clone()
        .ok_or_else(|| "assistant not selected".to_string())?;

    let mut role_name = "UnionAIAssistant".to_string();
    let mut message = text.clone();
    if text.starts_with('@') {
        let parts = text.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 2 {
            role_name = parts[0].trim_start_matches('@').to_string();
            message = parts[1..].join(" ");
        }
    }

    let tool_prompt = build_unionai_tool_prompt();
    let system_prompt = input.system_prompt.as_deref().unwrap_or("").trim().to_string();
    let mut runtime = assistant.clone();
    let mut cwd = resolve_chat_cwd(get_state(&state), input.selected_team_id.as_deref());
    let mut context_pairs: Vec<(String, String)> = Vec::new();
    let is_union_assistant = role_name == "UnionAIAssistant";

    if let Some(team_id) = input.selected_team_id.clone() {
        cwd = resolve_chat_cwd(get_state(&state), Some(&team_id));
        let entries = list_shared_context_internal(get_state(&state), &team_id).unwrap_or_default();
        for entry in entries {
            context_pairs.push((entry.key, entry.value));
        }
        if role_name != "UnionAIAssistant" {
            runtime = resolve_role_runtime(state.clone(), &team_id, &role_name).unwrap_or(runtime.clone());
            let role_prompt = resolve_role_prompt(state.clone(), &team_id, &role_name).unwrap_or_default();
            if !role_prompt.is_empty() {
                context_pairs.push(("role_prompt".to_string(), role_prompt));
            }
        }
    }

    // Always inject cwd so agent can answer "what directory" without needing /context
    if !context_pairs.iter().any(|(k, _)| k == "cwd") {
        context_pairs.insert(0, ("cwd".to_string(), cwd.clone()));
    }

    let mut parts = Vec::new();
    if !system_prompt.is_empty() {
        parts.push(format!("System:\n{}", system_prompt));
    }
    if is_union_assistant {
        parts.push(format!("Tools:\n{}", tool_prompt));
    }
    if !context_pairs.is_empty() {
        let ctx = context_pairs.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join("\n");
        parts.push(format!("Context:\n{}", ctx));
    }
    parts.push(format!("User:\n{}", message));
    let prepared = parts.join("\n\n");

    chat_log(
        "route.acp.start",
        json!({
            "runtime": runtime.clone(),
            "role": role_name.clone(),
            "cwd": cwd.clone(),
            "contextCount": context_pairs.len(),
            "preparedSize": prepared.len()
        }),
    );
    let acp_started = Instant::now();
    let llm = acp::execute_runtime(&runtime, &role_name, &prepared, &context_pairs, &cwd, &app).await;
    let output = llm.output.trim().to_string();
    let llm_delta_count = llm.deltas.len();
    let llm_meta = llm.meta.clone();
    chat_log(
        "route.acp.done",
        json!({
            "runtime": runtime.clone(),
            "role": role_name.clone(),
            "latencyMs": acp_started.elapsed().as_millis(),
            "totalLatencyMs": started.elapsed().as_millis(),
            "outputSize": output.len(),
            "deltaCount": llm_delta_count,
            "meta": llm_meta
        }),
    );
    let signals = detect_reply_signals(&output);
    if !signals.is_empty() {
        chat_log(
            "route.acp.signal",
            json!({
                "signals": signals,
                "preview": clip_text(&output, 220)
            }),
        );
    }

    let command_output = extract_command_output(&output);
    if let Some(command_text) = command_output {
        chat_log(
            "route.acp.command_output",
            json!({
                "command": clip_text(&command_text, 180)
            }),
        );
        let command_result = apply_chat_command(
            app,
            state,
            command_text.clone(),
            input.selected_team_id.clone(),
            Some(assistant.clone()),
        )
        .await?;
        return Ok(AssistantChatResponse {
            ok: command_result.ok,
            reply: format!("{}\n{}", command_text, command_result.message),
            selected_team_id: command_result.selected_team_id.clone(),
            selected_assistant: command_result.selected_assistant.clone(),
            session_id: command_result.session_id.clone(),
            command_result: Some(command_result),
        });
    }

    Ok(AssistantChatResponse {
        ok: true,
        reply: output,
        selected_team_id: input.selected_team_id,
        selected_assistant: Some(assistant),
        session_id: None,
        command_result: None,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Ok(path) = std::env::var("PATH") {
        let home = std::env::var("HOME").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!(
                "/opt/homebrew/bin:/usr/local/bin:{}/.npm-global/bin:{}/.bun/bin:{}/.cargo/bin:{}",
                home, home, home, path
            ),
        );
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            let app_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("unionai.sqlite3");
            let conn = Connection::open(db_path)?;
            init_db(&conn).map_err(std::io::Error::other)?;
            let state = AppState {
                db: Mutex::new(conn),
                shared_context: DashMap::new(),
            };

            {
                let existing = {
                    let guard = state.db.lock().map_err(|e| std::io::Error::other(e.to_string()))?;
                    let mut stmt = guard.prepare(
                        "SELECT team_id, key, value FROM shared_context_snapshots ORDER BY updated_at DESC",
                    )?;
                    let rows = stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })?;
                    let mut entries = Vec::new();
                    for row in rows {
                        entries.push(row?);
                    }
                    entries
                };

                for (team_id, key, value) in existing {
                    state.shared_context.insert(shared_key(&team_id, &key), value);
                }
            }

            app.manage(state);

            // Pre-warm all available ACP sessions in background so first chat is instant.
            // 1. Prewarm each catalog runtime (UnionAIAssistant slot)
            // 2. Prewarm each saved role so @Role has no cold start
            let app_state: &AppState = app.state::<AppState>().inner();
            let prewarm_cwd = default_chat_cwd();
            let catalog_snapshot: Vec<(String, bool)> = assistant_catalog()
                .iter()
                .map(|a| (a.key.clone(), a.available))
                .collect();
            // Collect all roles from all teams for prewarm
            let all_roles: Vec<(String, String, String)> = with_db(app_state, |conn| {
                let mut stmt = conn.prepare(
                    "SELECT r.role_name, r.runtime_kind, COALESCE(t.workspace_path, '') as wp
                     FROM roles r JOIN teams t ON r.team_id = t.id"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                }).map_err(|e| e.to_string())?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }).unwrap_or_default();
            let default_cwd = prewarm_cwd.clone();
            tauri::async_runtime::spawn(async move {
                for (key, available) in catalog_snapshot {
                    if available {
                        acp::prewarm(&key, &prewarm_cwd).await;
                    }
                }
                for (role_name, runtime_kind, workspace_path) in all_roles {
                    let cwd = abs_cwd(if workspace_path.is_empty() { &default_cwd } else { &workspace_path });
                    acp::prewarm_role(&runtime_kind, &role_name, &cwd).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_team,
            list_teams,
            upsert_role_cmd,
            list_roles,
            create_workflow,
            list_workflows,
            list_sessions,
            list_session_events,
            set_shared_context,
            get_shared_context,
            list_shared_context,
            start_workflow,
            apply_chat_command,
            detect_assistants,
            assistant_chat
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
