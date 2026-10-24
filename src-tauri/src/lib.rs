mod acp;
mod app_paths;
mod assistant;
mod chat;
mod commands;
mod credentials;
mod db;
mod engine;
mod error;
mod fs_context;
mod git;
pub mod importer;
pub mod jockey_mcp;
mod llm_providers;
mod parser;
mod resource_limits;
mod runtime_kind;
mod runtime_profile;
mod types;
pub mod web_gateway;

use dashmap::DashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

use assistant::refresh_assistant_catalog;
use db::{init_db, seed_default_dynamic_catalog, DbPool};
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    resource_limits::prepare();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            let release_dir = app.path().app_local_data_dir()?;
            let app_dir = app_paths::resolve_data_dir(&release_dir);
            fs::create_dir_all(&app_dir)?;
            app_paths::seed_dev_database(&release_dir, &app_dir);
            eprintln!("[jockey] data dir: {}", app_dir.to_string_lossy());
            acp::set_app_data_dir(app_dir.clone());
            engine::install(app.handle());
            git::set_app_handle(app.handle().clone());

            let db_path = app_dir.join(app_paths::DB_FILE);
            // Per-connection PRAGMAs run by the pool on every new connection.
            const CONN_INIT_SQL: &str =
                "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;";
            let db_pool = DbPool::new(db_path, 8, CONN_INIT_SQL)
                .map_err(std::io::Error::other)?;
            {
                let conn = db_pool.get().map_err(std::io::Error::other)?;
                init_db(&conn).map_err(std::io::Error::other)?;
            }
            let state = AppState {
                db: db_pool,
                role_cache: std::sync::Arc::new(DashMap::new()),
            };

            seed_default_dynamic_catalog(&state).map_err(std::io::Error::other)?;
            db::role::seed_default_roles(&state).map_err(std::io::Error::other)?;
            if let Err(e) = engine::recover_after_restart(&state) {
                eprintln!("[engine] startup recovery failed: {e}");
            }
            {
                let conn = state
                    .db
                    .get()
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                db::profiles::load_custom_runtime_profiles(&conn)
                    .map_err(std::io::Error::other)?;
                llm_providers::load_snapshot(&conn).map_err(std::io::Error::other)?;
            }

            let bridge_state = std::sync::Arc::new(state.clone_refs());
            let bridge_state_clone = bridge_state.clone();
            let bridge_app = app.handle().clone();
            let bridge_app_for_start = bridge_app.clone();
            tauri::async_runtime::spawn(async move {
                match jockey_mcp::bridge::start_bridge(bridge_state_clone.clone(), bridge_app_for_start).await {
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

            // Discover installed CLIs without opening historical provider sessions. Runtime
            // configuration is fetched on demand by the active session or role editor.
            tauri::async_runtime::spawn(async {
                let _ = tokio::task::spawn_blocking(refresh_assistant_catalog).await;
            });

            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::role::upsert_role_cmd,
            db::role::reassign_role_project_cmd,
            db::role::delete_role_cmd,
            db::role::list_role_active_sessions_cmd,
            db::role::list_roles,
            db::workflow::create_workflow,
            db::workflow::list_workflows,
            db::workflow::delete_workflow,
            db::session::list_sessions,
            db::session::list_session_events,
            db::session::start_workflow,
            commands::completion::complete_mentions,
            assistant::detect_assistants,
            commands::runtime_cmd::cancel_acp_session,
            commands::runtime_cmd::cancel_acp_subagent,
            commands::runtime_cmd::steer_acp_session,
            commands::runtime_cmd::reset_acp_session,
            commands::runtime_cmd::reconnect_acp_session,
            commands::runtime_cmd::set_acp_mode,
            commands::runtime_cmd::set_acp_config_option,
            commands::session_context_cmd::list_session_context_entries_cmd,
            commands::session_context_cmd::set_session_context_entry_cmd,
            commands::session_context_cmd::delete_session_context_entry_cmd,
            commands::runtime_cmd::list_available_commands_cmd,
            commands::runtime_cmd::acp_metrics_snapshot_cmd,
            commands::runtime_cmd::acp_log_snapshot_cmd,
            commands::runtime_cmd::active_acp_connections_cmd,
            commands::runtime_cmd::respond_permission,
            commands::provider_session_cmd::list_provider_sessions_cmd,
            commands::provider_session_cmd::import_provider_session_cmd,
            commands::provider_session_cmd::fork_provider_session_cmd,
            commands::provider_session_cmd::rewind_provider_session_cmd,
            commands::runtime_cmd::sync_role_mode_cmd,
            commands::runtime_cmd::prewarm_role_config_cmd,
            llm_providers::list_runtime_providers_cmd,
            llm_providers::get_llm_provider_configs_cmd,
            llm_providers::set_llm_provider_config_cmd,
            llm_providers::clear_llm_provider_config_cmd,
            credentials::list_agent_credentials_cmd,
            credentials::create_agent_credential_cmd,
            credentials::update_agent_credential_cmd,
            credentials::delete_agent_credential_cmd,
            credentials::get_role_credentials_cmd,
            credentials::set_role_credential_cmd,
            credentials::open_agent_credential_login_cmd,
            commands::resume_cmd::open_session_in_terminal_cmd,
            commands::runtime_cmd::bind_session_agent_cmd,
            commands::runtime_cmd::respond_user_input_cmd,
            commands::runtime_cmd::list_pending_prompts_cmd,
            commands::runtime_cmd::get_project_agent_config_cmd,
            commands::runtime_cmd::list_project_agent_pins_cmd,
            commands::runtime_cmd::set_project_agent_config_cmd,
            commands::runtime_cmd::set_project_agent_runtime_cmd,
            commands::git_cmd::git_status_cmd,
            commands::git_cmd::git_diff_cmd,
            commands::git_cmd::git_file_cmd,
            commands::git_cmd::git_list_branches_cmd,
            commands::git_cmd::git_checkout_cmd,
            commands::git_cmd::git_pr_url_cmd,
            commands::git_cmd::git_remote_info_cmd,
            commands::git_cmd::git_commit_cmd,
            commands::git_cmd::git_stage_cmd,
            commands::git_cmd::git_unstage_cmd,
            commands::git_cmd::git_fetch_cmd,
            commands::git_cmd::git_pull_cmd,
            commands::git_cmd::git_push_cmd,
            commands::git_cmd::git_create_pr_cmd,
            commands::git_cmd::git_log_cmd,
            commands::git_cmd::git_commit_diff_cmd,
            commands::git_cmd::git_commit_detail_cmd,
            commands::git_cmd::git_commit_file_diff_cmd,
            commands::fs_cmd::list_dir_cmd,
            commands::fs_cmd::read_file_base64_cmd,
            db::app_session::list_app_sessions,
            db::app_session::list_closed_app_sessions,
            engine::commands::create_app_session,
            engine::commands::fork_app_session,
            engine::commands::update_app_session,
            engine::commands::delete_app_session,
            engine::commands::reopen_app_session,
            engine::commands::get_app_sessions,
            engine::commands::list_session_summaries_cmd,
            engine::commands::watch_session,
            engine::commands::unwatch_session,
            engine::commands::get_live_turn,
            commands::cwd_util::register_draft_session_cwd,
            db::app_session::append_app_message,
            db::app_session::save_app_message,
            db::inbox::list_session_inbox_cmd,
            db::inbox::enqueue_session_inbox_cmd,
            db::inbox::remove_session_inbox_cmd,
            db::inbox::claim_session_inbox_cmd,
            db::inbox::restore_session_inbox_cmd,
            db::lifecycle::list_agent_lifecycle_cmd,
            db::skill::list_app_skills,
            db::skill::upsert_app_skill,
            db::skill::delete_app_skill,
            db::skill::list_all_skills_for_role_cmd,
            db::skill::set_role_skills_cmd,
            db::global_mcp::list_global_mcp_servers_cmd,
            db::global_mcp::upsert_global_mcp_server_cmd,
            db::global_mcp::delete_global_mcp_server_cmd,
            db::global_mcp::list_role_mcp_servers_cmd,
            db::global_mcp::set_role_mcp_enabled_cmd,
            db::profiles::list_runtime_profiles_cmd,
            db::profiles::upsert_runtime_profile_cmd,
            db::profiles::delete_runtime_profile_cmd,
            commands::runtime_cmd::reset_role_mcp_sessions_cmd,
            commands::terminal_cmd::start_terminal_session,
            commands::terminal_cmd::resize_terminal_session,
            commands::terminal_cmd::write_terminal_session,
            commands::terminal_cmd::stop_terminal_session,
            commands::workspace_cmd::open_workspace_cmd,
            commands::workspace_cmd::get_workspace_app_icon_cmd,
            commands::project_cmd::list_projects_cmd,
            commands::project_cmd::get_project_cmd,
            engine::commands::create_project_cmd,
            engine::commands::delete_project_cmd,
            engine::commands::import_project_sessions_cmd,
            engine::commands::import_cli_sessions_cmd,
            commands::project_cmd::scan_all_importable_sessions_cmd,
            commands::project_cmd::scan_importable_sessions_cmd,
            db::rule::list_rules_cmd,
            db::rule::upsert_rule_cmd,
            db::rule::delete_rule_cmd,
            db::rule::set_role_rules_cmd,
            db::rule::list_role_rules_cmd,
            db::rule::list_all_rules_for_role_cmd,
            engine::service::submit_input,
            engine::service::cancel_turn,
            engine::service::send_queued_input_now,
            engine::service::remove_queued_input,
            web_gateway::get_gateway_status_cmd,
            web_gateway::start_gateway_cmd,
            web_gateway::stop_gateway_cmd,
            web_gateway::regenerate_gateway_token_cmd
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
