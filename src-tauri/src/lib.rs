mod acp;
mod assistant;
mod chat;
mod commands;
mod db;
mod types;

use dashmap::DashMap;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::fs;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

use assistant::assistant_catalog;
use db::context::shared_key;
use db::{
    ensure_default_team_id, init_db, load_team_workspace_path, seed_default_dynamic_catalog,
    with_db,
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
To control UnionAI, suggest a slash command on its own line. Commands are not auto-executed.\n\
Prefer direct assistant or @role chat for simple/single-role tasks.\n\
Use workflow commands only when the task is explicitly multi-step or cross-role.\n\
  /assistant list | /assistant select <runtime>\n\
  /model list | /model add <model> | /model remove <model>\n\
  /model select <model> | /model select role <name> <model> | /model get | /model clear\n\
  /mcp list | /mcp add <name> | /mcp remove <name> | /mcp enable <name> | /mcp disable <name>\n\
  /skill list | /skill add <name> | /skill remove <name> | /skill enable <name> | /skill disable <name>\n\
  /role list | /role bind <role> <runtime> [prompt] | /role prompt <role> <prompt> | /role delete <role>\n\
  /workflow list | /workflow create <name> <r1,r2> | /workflow start <name> <prompt>\n\
  /session list | /session stop <id> | /session reset assistant | /session reset role <name>\n\
  /context list | /context list role <name>\n\
  /context set <key> <value> | /context set role <name> <key> <value>\n\
  /context get <key> | /context get role <name> <key>\n\
  /context delete <key> | /context delete role <name> <key>\n\
  /run <prompt>\n\
Workspace selection is managed by the app automatically.\n\
These are UI commands for UnionAI, NOT tools you can invoke. Only output them when the user explicitly asks to perform a UnionAI action."
}

#[tauri::command]
async fn cancel_acp_session(runtime_kind: String, role_name: String) -> Result<(), String> {
    acp::cancel_session(&runtime_kind, &role_name).await;
    Ok(())
}

#[tauri::command]
async fn set_acp_mode(
    runtime_kind: String,
    role_name: String,
    mode_id: String,
) -> Result<(), String> {
    acp::set_mode(&runtime_kind, &role_name, &mode_id).await
}

#[tauri::command]
async fn set_acp_config_option(
    runtime_kind: String,
    role_name: String,
    config_id: String,
    value: String,
) -> Result<(), String> {
    acp::set_config_option(&runtime_kind, &role_name, &config_id, &value).await
}

#[tauri::command]
fn list_available_commands_cmd(runtime_key: String) -> Vec<serde_json::Value> {
    acp::list_available_commands(&runtime_key)
}

#[tauri::command]
fn list_discovered_config_options_cmd(runtime_key: String) -> Vec<serde_json::Value> {
    acp::list_discovered_config_options(&runtime_key)
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
            let all_roles: Vec<(String, String, String)> = with_db(app_state, |conn| {
                let mut stmt = conn.prepare(
                    "SELECT r.role_name, r.runtime_kind, COALESCE(t.workspace_path, '') as wp
                     FROM roles r JOIN teams t ON r.team_id = t.id
                     ORDER BY r.updated_at DESC
                     LIMIT ?1"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map(params![PREWARM_ROLE_LIMIT as i64], |row| {
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
                    if runtime_kind != "mock" && !available_runtime_keys.contains(&runtime_kind) {
                        continue;
                    }
                    let cwd = abs_cwd(if workspace_path.is_empty() { &default_cwd } else { &workspace_path });
                    acp::prewarm_role(&runtime_kind, &role_name, &cwd).await;
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
            respond_permission
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
