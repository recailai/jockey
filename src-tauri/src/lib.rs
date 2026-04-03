mod acp;
mod assistant;
mod chat;
mod commands;
mod db;
mod error;
mod fs_context;
pub mod jockey_mcp;
mod parser;
mod runtime_kind;
mod types;

use dashmap::DashMap;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

use assistant::refresh_assistant_catalog;
use db::context::shared_key;
use db::{init_db, seed_default_dynamic_catalog, with_db, DbPool};
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

pub(crate) fn build_jockey_tool_prompt() -> &'static str {
    "You are Jockey assistant. Answer the user's question directly and concisely.\n\
IMPORTANT: Do NOT use any tools, read files, run commands, or explore the filesystem.\n\
\n\
App commands (prefix /app_) — only suggest when the user explicitly asks for Jockey management:\n\
  /app_help | /app_assistant list | /app_assistant select <runtime>\n\
Do NOT suggest role, model, or MCP commands — those are managed via the UI sidebar."
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            let app_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&app_dir)?;
            acp::set_app_data_dir(app_dir.clone());

            let db_path = app_dir.join("jockey.sqlite3");
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
                role_cache: std::sync::Arc::new(DashMap::new()),
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

            let bridge_state = std::sync::Arc::new(AppState {
                db: state.db.clone(),
                shared_context: state.shared_context.clone(),
                role_cache: state.role_cache.clone(),
            });
            let bridge_state_clone = bridge_state.clone();
            let bridge_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match jockey_mcp::bridge::start_bridge(bridge_state_clone.clone()).await {
                    Ok((port, token)) => {
                        eprintln!("[jockey-mcp] listening on 127.0.0.1:{port}");
                        db::global_mcp::seed_builtin_jockey_mcp(&bridge_state_clone, port, &token);
                    }
                    Err(e) => {
                        eprintln!("[jockey-mcp] failed to start: {e}");
                        let _ = bridge_app.emit("jockey-mcp/error", &e);
                    }
                }
            });

            app.manage(state);

            let (death_tx, mut death_rx) = tokio::sync::mpsc::unbounded_channel::<acp::ConnectionDeathEvent>();
            acp::set_death_event_sender(death_tx);
            let death_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = death_rx.recv().await {
                    let _ = death_app.emit("acp/connection-lost", &event);
                }
            });

            let (prewarm_tx, mut prewarm_rx) = tokio::sync::mpsc::unbounded_channel::<acp::PrewarmEvent>();
            acp::set_prewarm_event_sender(prewarm_tx);
            let prewarm_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = prewarm_rx.recv().await {
                    let _ = prewarm_app.emit("acp/prewarm", &event);
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
                    let tmp = app_state_inner.clone_refs();
                    let sid_clone = app_session_id.clone();
                    let role_name_clone = role_name.clone();
                    let runtime_kind_clone = runtime_kind.clone();
                    let resume_sid_clone = resume_sid.clone();
                    priority_futs.push(Box::pin(async move {
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
                        }).unwrap_or_else(|e| { eprintln!("[refresh] list session roles error: {e}"); vec![] });

                        let refresh_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
                        let mut active_role_names: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        for (session_id, role_name, runtime_kind) in session_roles {
                            if runtime_kind == "mock" { continue; }
                            active_role_names.insert(role_name.clone());
                            let tmp = app_state.clone_refs();
                            let sid_clone = session_id.clone();
                            let rn_clone = role_name.clone();
                            let rk_clone = runtime_kind.clone();
                            let permit = refresh_sem.clone().acquire_owned().await.ok();
                            tokio::spawn(async move {
                                let _permit = permit;
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

                        let all_roles: Vec<(String, String)> = with_db(app_state, |conn| {
                            let mut stmt = conn
                                .prepare("SELECT role_name, runtime_kind FROM roles ORDER BY role_name ASC")
                                .map_err(|e| e.to_string())?;
                            let rows = stmt
                                .query_map([], |row| {
                                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                                })
                                .map_err(|e| e.to_string())?;
                            Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                        })
                        .unwrap_or_else(|e| { eprintln!("[refresh] list roles error: {e}"); vec![] });

                        for (role_name, runtime_kind) in all_roles {
                            if runtime_kind == "mock" {
                                continue;
                            }
                            if active_role_names.contains(&role_name) {
                                continue;
                            }
                            let tmp = app_state.clone_refs();
                            let rn_clone = role_name.clone();
                            let rk_clone = runtime_kind.clone();
                            let permit = refresh_sem.clone().acquire_owned().await.ok();
                            tokio::spawn(async move {
                                let _permit = permit;
                                let cwd = resolve_chat_cwd();
                                acp::refresh_role_config_defs(&rk_clone, &rn_clone, &cwd, &tmp)
                                    .await;
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
            db::workflow::delete_workflow,
            db::session::list_sessions,
            db::session::list_session_events,
            db::session::start_workflow,
            commands::completion::complete_mentions,
            commands::completion::complete_cli,
            commands::apply_chat_command,
            assistant::detect_assistants,
            chat::assistant_chat,
            commands::runtime_cmd::cancel_acp_session,
            commands::runtime_cmd::reset_acp_session,
            commands::runtime_cmd::reconnect_acp_session,
            commands::runtime_cmd::set_acp_mode,
            commands::runtime_cmd::set_acp_config_option,
            commands::session_context_cmd::list_session_context_entries_cmd,
            commands::session_context_cmd::set_session_context_entry_cmd,
            commands::session_context_cmd::delete_session_context_entry_cmd,
            commands::runtime_cmd::list_discovered_config_options_cmd,
            commands::runtime_cmd::list_discovered_modes_cmd,
            commands::runtime_cmd::list_available_commands_cmd,
            commands::runtime_cmd::respond_permission,
            commands::runtime_cmd::prewarm_role_config_cmd,
            db::app_session::list_app_sessions,
            db::app_session::list_closed_app_sessions,
            db::app_session::create_app_session,
            db::app_session::update_app_session,
            db::app_session::delete_app_session,
            db::app_session::reopen_app_session,
            db::app_session::append_app_message,
            db::skill::list_app_skills,
            db::skill::upsert_app_skill,
            db::skill::delete_app_skill,
            db::global_mcp::list_global_mcp_servers_cmd,
            db::global_mcp::upsert_global_mcp_server_cmd,
            db::global_mcp::delete_global_mcp_server_cmd
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
