mod acp;
mod assistant;
mod chat;
mod commands;
mod db;
mod error;
mod fs_context;
mod parser;
mod runtime_kind;
mod types;

use dashmap::DashMap;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager, State};

use assistant::refresh_assistant_catalog;
use assistant::normalize_runtime_key;
use db::context::{clear_shared_context_internal, set_shared_context_internal, shared_key};
use db::session_context::{app_session_scope, list_shared_context_prefix_internal};
use db::{get_state, init_db, seed_default_dynamic_catalog, with_db, DbPool};
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

pub(crate) fn resolve_chat_cwd() -> String {
    abs_cwd(&default_chat_cwd())
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
  /app_role list | /app_role bind <role> <runtime> [prompt] | /app_role prompt <role> <prompt>\n\
Workspace selection is managed by the app automatically.\n\
Only output app commands when the user explicitly asks to perform a UnionAI action."
}

#[tauri::command]
async fn cancel_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::cancel_session(&runtime, &role_name, Some(app_session_id.as_str())).await;
    Ok(())
}

#[tauri::command]
async fn reset_acp_session(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::reset_session(&runtime, &role_name, Some(app_session_id.as_str())).await?;
    crate::db::app_session_role::clear_app_session_role_cli_id(
        get_state(&state),
        &app_session_id,
        &role_name,
    )?;
    Ok(())
}

#[tauri::command]
async fn set_acp_mode(
    state: State<'_, AppState>,
    role_name: String,
    mode_id: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::set_mode(
        &runtime,
        &role_name,
        &mode_id,
        Some(app_session_id.as_str()),
    )
    .await
}

#[tauri::command]
async fn set_acp_config_option(
    state: State<'_, AppState>,
    role_name: String,
    config_id: String,
    value: String,
    app_session_id: String,
) -> Result<(), String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    acp::set_config_option(
        &runtime,
        &role_name,
        &config_id,
        &value,
        Some(app_session_id.as_str()),
    )
    .await
}

#[tauri::command]
fn list_available_commands_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Vec<serde_json::Value> {
    if app_session_id.trim().is_empty() {
        return vec![];
    }
    let runtime = match resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    acp::list_available_commands(&app_session_id, &runtime, &role_name)
}

#[tauri::command]
fn list_discovered_config_options_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Vec<serde_json::Value> {
    if app_session_id.trim().is_empty() {
        return vec![];
    }
    let runtime = match resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    acp::list_discovered_config_options(&app_session_id, &runtime, &role_name)
}

fn ensure_scope_in_app_session(app_session_id: &str, scope: &str) -> Result<(), String> {
    let root = app_session_scope(app_session_id);
    let child_prefix = format!("{root}:");
    if scope == root || scope.starts_with(&child_prefix) {
        return Ok(());
    }
    Err(format!("scope must stay within {root}"))
}

#[tauri::command]
fn list_session_context_entries_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
) -> Result<Vec<ContextEntry>, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }
    let root = app_session_scope(sid);
    let child_prefix = format!("{root}:");
    list_shared_context_prefix_internal(get_state(&state), &root).map(|items| {
        items
            .into_iter()
            .filter(|entry| entry.scope == root || entry.scope.starts_with(&child_prefix))
            .collect()
    })
}

#[tauri::command]
fn set_session_context_entry_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
    scope: String,
    key: String,
    value: String,
) -> Result<ContextEntry, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }
    let scope = scope.trim();
    let key = key.trim();
    if scope.is_empty() || key.is_empty() {
        return Err("scope/key required".to_string());
    }
    ensure_scope_in_app_session(sid, scope)?;
    set_shared_context_internal(get_state(&state), scope, key, value.trim())
}

#[tauri::command]
fn delete_session_context_entry_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
    scope: String,
    key: String,
) -> Result<(), String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }
    let scope = scope.trim();
    let key = key.trim();
    if scope.is_empty() || key.is_empty() {
        return Err("scope/key required".to_string());
    }
    ensure_scope_in_app_session(sid, scope)?;
    clear_shared_context_internal(get_state(&state), scope, key)
}

#[tauri::command]
async fn prewarm_role_config_cmd(
    state: State<'_, AppState>,
    role_name: String,
    app_session_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    if app_session_id.trim().is_empty() {
        return Err("app session id required".to_string());
    }
    let runtime = resolve_runtime_for_session_role(get_state(&state), &app_session_id, &role_name)?;
    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), &app_session_id)
        .unwrap_or_else(resolve_chat_cwd);
    Ok(acp::prewarm_role_for_config(
        &runtime,
        &role_name,
        &cwd,
        Some((get_state(&state), &app_session_id)),
    )
    .await)
}

fn normalize_runtime_or_self(runtime: &str) -> String {
    normalize_runtime_key(runtime)
        .unwrap_or(runtime)
        .to_string()
}

fn resolve_runtime_for_session_role(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
) -> Result<String, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }

    if role_name == "UnionAIAssistant" {
        let session_runtime = with_db(state, |conn| {
            conn.query_row(
                "SELECT runtime_kind FROM app_sessions WHERE id = ?1",
                params![sid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
            .map(|v| v.flatten())
        })?;
        if let Some(rt) = session_runtime {
            return Ok(normalize_runtime_or_self(&rt));
        }
        return Err("assistant runtime not selected".to_string());
    }

    if let Some(row) = crate::db::app_session_role::load_app_session_role_state(state, sid, role_name)? {
        if let Some(rt) = row.runtime_kind {
            return Ok(normalize_runtime_or_self(&rt));
        }
    }
    Ok(normalize_runtime_or_self(&crate::db::role::load_role_runtime_kind(
        state, role_name,
    )?))
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
    let app = tauri::Builder::default()
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
                        "SELECT scope, key, value FROM shared_context_snapshots ORDER BY updated_at DESC",
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

                for (scope, key, value) in existing {
                    state.shared_context.insert(shared_key(&scope, &key), value);
                }
            }

            seed_default_dynamic_catalog(&state).map_err(std::io::Error::other)?;
            app.manage(state);

            let (death_tx, mut death_rx) = tokio::sync::mpsc::unbounded_channel::<acp::ConnectionDeathEvent>();
            acp::set_death_event_sender(death_tx);
            let death_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = death_rx.recv().await {
                    let _ = death_app.emit("acp/connection-lost", &event);
                }
            });

            let catalog_snapshot: Vec<(String, bool)> = refresh_assistant_catalog()
                .iter()
                .map(|a| (a.key.clone(), a.available))
                .collect();
            let available_runtime_keys: HashSet<String> = catalog_snapshot
                .iter()
                .filter_map(|(key, available)| if *available { Some(key.clone()) } else { None })
                .collect();

            // Load recent session's role mappings from app_session_roles
            let app_state: &AppState = app.state::<AppState>().inner();
            let recent_app_session_id: Option<String> = with_db(app_state, |conn| {
                conn.query_row(
                    "SELECT id FROM app_sessions ORDER BY last_active_at DESC LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())
            }).unwrap_or(None);

            let recent_role_mappings: Vec<(String, String, Option<String>)> = if let Some(ref sid) = recent_app_session_id {
                with_db(app_state, |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT role_name, runtime_kind, acp_session_id FROM app_session_roles WHERE app_session_id = ?1"
                    ).map_err(|e| e.to_string())?;
                    let rows = stmt.query_map(params![sid], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
                    }).map_err(|e| e.to_string())?;
                    Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                }).unwrap_or_default()
            } else {
                Vec::new()
            };

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut priority_futs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> = Vec::new();
                let app_session_id = match recent_app_session_id.clone() {
                    Some(sid) => sid,
                    None => return,
                };

                for (role_name, runtime_kind, resume_sid) in &recent_role_mappings {
                    if runtime_kind != "mock" && !available_runtime_keys.contains(runtime_kind) {
                        continue;
                    }
                    let cwd = {
                        let state_ref = app_handle.state::<AppState>();
                        crate::db::app_session::get_app_session_cwd(state_ref.inner(), &app_session_id)
                            .unwrap_or_else(default_chat_cwd)
                    };
                    let state_ref = app_handle.state::<AppState>();
                    let app_state_inner: &AppState = state_ref.inner();
                    let db_clone = app_state_inner.db.clone();
                    let ctx_clone = app_state_inner.shared_context.clone();
                    let sid_clone = app_session_id.clone();
                    let role_name_clone = role_name.clone();
                    let runtime_kind_clone = runtime_kind.clone();
                    let resume_sid_clone = resume_sid.clone();
                    priority_futs.push(Box::pin(async move {
                        let tmp = AppState { db: db_clone, shared_context: ctx_clone };
                        acp::prewarm_role_with_session_id(&runtime_kind_clone, &role_name_clone, &cwd, resume_sid_clone, &tmp, &sid_clone).await;
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
            });

            // Background config-options refresh: every 5 minutes, re-prewarm all active
            // session roles so the in-memory cache stays current without user interaction.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                    interval.tick().await; // skip immediate first tick
                    loop {
                        interval.tick().await;
                        let state_ref = app_handle.state::<AppState>();
                        let app_state: &AppState = state_ref.inner();
                        // Collect all active (non-closed) sessions and their role mappings
                        let session_roles: Vec<(String, String, String)> = with_db(app_state, |conn| {
                            let mut stmt = conn.prepare(
                                "SELECT r.app_session_id, r.role_name, r.runtime_kind
                                 FROM app_session_roles r
                                 JOIN app_sessions s ON s.id = r.app_session_id
                                 WHERE s.closed_at IS NULL",
                            ).map_err(|e| e.to_string())?;
                            let rows = stmt.query_map([], |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                ))
                            }).map_err(|e| e.to_string())?;
                            Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                        }).unwrap_or_default();

                        for (session_id, role_name, runtime_kind) in session_roles {
                            if runtime_kind == "mock" { continue; }
                            let db_clone = app_state.db.clone();
                            let ctx_clone = app_state.shared_context.clone();
                            let sid_clone = session_id.clone();
                            let rn_clone = role_name.clone();
                            let rk_clone = runtime_kind.clone();
                            tokio::spawn(async move {
                                let tmp = AppState { db: db_clone, shared_context: ctx_clone };
                                let cwd = crate::db::app_session::get_app_session_cwd(&tmp, &sid_clone)
                                    .unwrap_or_else(resolve_chat_cwd);
                                acp::prewarm_role_for_config(
                                    &rk_clone,
                                    &rn_clone,
                                    &cwd,
                                    Some((&tmp, &sid_clone)),
                                ).await;
                            });
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::role::upsert_role_cmd,
            db::role::delete_role_cmd,
            db::role::list_roles,
            db::workflow::create_workflow,
            db::workflow::list_workflows,
            db::session::list_sessions,
            db::session::list_session_events,
            db::session::start_workflow,
            commands::completion::complete_mentions,
            commands::completion::complete_cli,
            commands::apply_chat_command,
            assistant::detect_assistants,
            chat::assistant_chat,
            cancel_acp_session,
            reset_acp_session,
            set_acp_mode,
            set_acp_config_option,
            list_session_context_entries_cmd,
            set_session_context_entry_cmd,
            delete_session_context_entry_cmd,
            list_discovered_config_options_cmd,
            list_available_commands_cmd,
            respond_permission,
            prewarm_role_config_cmd,
            db::app_session::list_app_sessions,
            db::app_session::list_closed_app_sessions,
            db::app_session::create_app_session,
            db::app_session::update_app_session,
            db::app_session::delete_app_session,
            db::app_session::reopen_app_session,
            db::app_session::append_app_message,
            db::skill::list_app_skills,
            db::skill::upsert_app_skill,
            db::skill::delete_app_skill
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    let mut did_shutdown = false;
    app.run(move |_app_handle, event| {
        if did_shutdown {
            return;
        }
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            did_shutdown = true;
            tauri::async_runtime::block_on(async {
                acp::shutdown().await;
            });
        }
    });
}
