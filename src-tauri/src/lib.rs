mod acp;
mod assistant;
mod chat;
mod commands;
mod db;
mod fs_context;
mod parser;
mod runtime_kind;
mod types;

use dashmap::DashMap;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

use assistant::assistant_catalog;
use db::context::shared_key;
use db::{
    ensure_default_team_id, init_db, load_team_workspace_path, seed_default_dynamic_catalog,
    with_db, DbPool,
};
use types::*;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn default_chat_cwd() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/tmp".to_string())
        })
}

pub(crate) fn abs_cwd(path: &str) -> String {
    let p = std::path::Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::PathBuf::from(default_chat_cwd()).join(p)
    };
    abs.canonicalize()
        .unwrap_or(abs)
        .to_string_lossy()
        .to_string()
}

pub(crate) fn resolve_chat_cwd(state: &AppState, team_id: Option<&str>) -> String {
    let raw = if let Some(team) = team_id {
        load_team_workspace_path(state, team).unwrap_or_else(|_| default_chat_cwd())
    } else {
        default_chat_cwd()
    };
    abs_cwd(&raw)
}

pub(crate) fn clip_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>()
}

pub(crate) fn build_unionai_tool_prompt() -> &'static str {
    "You are UnionAI assistant. Answer the user's question directly and concisely.\n\
IMPORTANT: Do NOT use any tools, read files, run commands, or explore the filesystem.\n\
\n\
App commands (prefix /app_) control UnionAI itself. Suggest them on their own line — not auto-executed.\n\
  /app_help\n\
  /app_assistant list | /app_assistant select <runtime>\n\
  /app_model list | /app_model add <model> | /app_model remove <model>\n\
  /app_model select <model> | /app_model select role <name> <model> | /app_model get | /app_model clear\n\
  /app_mcp list | /app_mcp add <name> | /app_mcp remove <name> | /app_mcp enable <name> | /app_mcp disable <name>\n\
  /app_role list | /app_role bind <role> <runtime> [prompt] | /app_role prompt <role> <prompt> | /app_role delete <role>\n\
  /app_role edit <role> mode <mode> | /app_role edit <role> model <model> | /app_role copy <src> <dst>\n\
Workspace selection is managed by the app automatically.\n\
Only output app commands when the user explicitly asks to perform a UnionAI action."
}

#[tauri::command]
async fn cancel_acp_session(runtime_kind: String, role_name: String, app_session_id: Option<String>) -> Result<(), String> {
    acp::cancel_session(&runtime_kind, &role_name, app_session_id.as_deref().unwrap_or("")).await;
    Ok(())
}

#[tauri::command]
async fn set_acp_mode(
    runtime_kind: String,
    role_name: String,
    app_session_id: Option<String>,
    mode_id: String,
) -> Result<(), String> {
    acp::set_mode(&runtime_kind, &role_name, app_session_id.as_deref().unwrap_or(""), &mode_id).await
}

#[tauri::command]
async fn set_acp_config_option(
    runtime_kind: String,
    role_name: String,
    app_session_id: Option<String>,
    config_id: String,
    value: String,
) -> Result<(), String> {
    acp::set_config_option(&runtime_kind, &role_name, app_session_id.as_deref().unwrap_or(""), &config_id, &value).await
}

#[tauri::command]
fn list_available_commands_cmd(runtime_key: String, role_name: String) -> Vec<serde_json::Value> {
    acp::list_available_commands(&runtime_key, &role_name)
}

#[tauri::command]
fn list_discovered_config_options_cmd(runtime_key: String) -> Vec<serde_json::Value> {
    acp::list_discovered_config_options(&runtime_key)
}

#[tauri::command]
async fn prewarm_role_config_cmd(
    state: tauri::State<'_, AppState>,
    runtime_kind: String,
    role_name: String,
    team_id: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let cwd = resolve_chat_cwd(&state, team_id.as_deref());
    let resolved_team_id = team_id.unwrap_or_default();
    let state_inner = state.inner();
    Ok(acp::prewarm_role_for_config(
        &runtime_kind,
        &role_name,
        &cwd,
        Some((state_inner, &resolved_team_id)),
    )
    .await)
}

#[tauri::command]
async fn respond_permission(
    request_id: String,
    option_id: String,
    cancelled: bool,
) -> Result<(), String> {
    use agent_client_protocol as acpsdk;
    let outcome = if cancelled {
        acpsdk::RequestPermissionOutcome::Cancelled
    } else {
        acpsdk::RequestPermissionOutcome::Selected(acpsdk::SelectedPermissionOutcome::new(
            acpsdk::PermissionOptionId::from(option_id),
        ))
    };
    acp::respond_to_permission(&request_id, outcome);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Ok(path) = std::env::var("PATH") {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut new_path = format!(
            "/usr/local/bin:{}/.npm-global/bin:{}/.bun/bin:{}/.cargo/bin:{}",
            home, home, home, path
        );
        if cfg!(target_os = "macos") {
            new_path = format!("/opt/homebrew/bin:{}", new_path);
        }
        std::env::set_var("PATH", new_path);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            let app_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&app_dir)?;
            acp::set_app_data_dir(app_dir.clone());
            let db_path = app_dir.join("unionai.sqlite3");
            // Per-connection PRAGMAs run by the pool on every new connection.
            const CONN_INIT_SQL: &str =
                "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;";
            let db_pool = DbPool::new(db_path, 8, CONN_INIT_SQL)
                .map_err(std::io::Error::other)?;
            {
                let conn = db_pool.get().map_err(std::io::Error::other)?;
                init_db(&conn).map_err(std::io::Error::other)?;
            }
            let state = AppState {
                db: db_pool,
                shared_context: DashMap::new(),
            };

            {
                let existing = {
                    let guard = state.db.get().map_err(|e| std::io::Error::other(e.to_string()))?;
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

            seed_default_dynamic_catalog(&state).map_err(std::io::Error::other)?;
            app.manage(state);

            let app_state: &AppState = app.state::<AppState>().inner();
            let _ = ensure_default_team_id(app_state);
            let prewarm_cwd = default_chat_cwd();
            let catalog_snapshot: Vec<(String, bool)> = assistant_catalog()
                .iter()
                .map(|a| (a.key.clone(), a.available))
                .collect();
            let available_runtime_keys: HashSet<String> = catalog_snapshot
                .iter()
                .filter_map(|(key, available)| if *available { Some(key.clone()) } else { None })
                .collect();
            let recent_app_session: Option<(String, String)> = with_db(app_state, |conn| {
                conn.query_row(
                    "SELECT id, role_sessions_json FROM app_sessions ORDER BY last_active_at DESC LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())
            }).unwrap_or(None);
            let all_roles: Vec<(String, String)> = with_db(app_state, |conn| {
                let mut stmt = conn.prepare(
                    "SELECT role_name, runtime_kind FROM roles ORDER BY updated_at DESC LIMIT ?1"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map(params![PREWARM_ROLE_LIMIT as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }).map_err(|e| e.to_string())?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }).unwrap_or_default();
            let app_handle = app.handle().clone();
            let default_cwd = prewarm_cwd.clone();
            tauri::async_runtime::spawn(async move {
                let role_sessions: serde_json::Map<String, serde_json::Value> =
                    recent_app_session.as_ref()
                        .and_then(|(_, json)| serde_json::from_str(json).ok())
                        .unwrap_or_default();
                let app_session_id = recent_app_session
                    .as_ref()
                    .map(|(id, _)| id.clone())
                    .unwrap_or_default();

                let recent_role_names: std::collections::HashSet<String> = role_sessions.keys().cloned().collect();

                let mut priority_futs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> = Vec::new();

                for (key, available) in &catalog_snapshot {
                    if *available {
                        let cwd = prewarm_cwd.clone();
                        let key = key.clone();
                        priority_futs.push(Box::pin(async move {
                            acp::prewarm(&key, &cwd).await;
                        }));
                    }
                }

                for (role_name, runtime_kind) in &all_roles {
                    if runtime_kind != "mock" && !available_runtime_keys.contains(runtime_kind) {
                        continue;
                    }
                    if !recent_role_names.contains(role_name) {
                        continue;
                    }
                    let cwd = abs_cwd(&default_cwd);
                    let state_ref = app_handle.state::<AppState>();
                    let app_state_inner: &AppState = state_ref.inner();
                    let db_clone = app_state_inner.db.clone();
                    let ctx_clone = app_state_inner.shared_context.clone();
                    let resume_sid = role_sessions.get(role_name).and_then(|v| v.as_str()).map(|s| s.to_string());
                    let sid_clone = app_session_id.clone();
                    let role_name_clone = role_name.clone();
                    let runtime_kind_clone = runtime_kind.clone();
                    priority_futs.push(Box::pin(async move {
                        let tmp = AppState { db: db_clone, shared_context: ctx_clone };
                        acp::prewarm_role_with_session_id(&runtime_kind_clone, &role_name_clone, &cwd, resume_sid, &tmp, &sid_clone).await;
                    }));
                }

                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
                let mut handles = Vec::new();
                for fut in priority_futs {
                    let permit = sem.clone().acquire_owned().await.ok();
                    handles.push(tokio::spawn(async move {
                        let _permit = permit;
                        fut.await;
                    }));
                }
                for h in handles {
                    let _ = h.await;
                }

                for (role_name, runtime_kind) in all_roles {
                    if runtime_kind != "mock" && !available_runtime_keys.contains(&runtime_kind) {
                        continue;
                    }
                    if recent_role_names.contains(&role_name) {
                        continue;
                    }
                    let cwd = abs_cwd(&default_cwd);
                    let state_ref = app_handle.state::<AppState>();
                    let app_state_inner: &AppState = state_ref.inner();
                    let db_clone = app_state_inner.db.clone();
                    let ctx_clone = app_state_inner.shared_context.clone();
                    let tmp = AppState { db: db_clone, shared_context: ctx_clone };
                    acp::prewarm_role_with_session_id(&runtime_kind, &role_name, &cwd, None, &tmp, "").await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::role::upsert_role_cmd,
            db::role::list_roles,
            db::workflow::create_workflow,
            db::workflow::list_workflows,
            db::session::list_sessions,
            db::session::list_session_events,
            db::context::set_shared_context,
            db::context::append_shared_context,
            db::context::get_shared_context,
            db::context::list_shared_context,
            db::session::start_workflow,
            commands::completion::complete_mentions,
            commands::completion::complete_cli,
            commands::apply_chat_command,
            assistant::detect_assistants,
            chat::assistant_chat,
            cancel_acp_session,
            set_acp_mode,
            set_acp_config_option,
            list_discovered_config_options_cmd,
            list_available_commands_cmd,
            respond_permission,
            prewarm_role_config_cmd,
            db::app_session::list_app_sessions,
            db::app_session::create_app_session,
            db::app_session::update_app_session,
            db::app_session::delete_app_session,
            db::app_session::append_app_message,
            db::skill::list_app_skills,
            db::skill::upsert_app_skill,
            db::skill::delete_app_skill
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
