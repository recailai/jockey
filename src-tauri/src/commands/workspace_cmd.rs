use std::process::Stdio;

use tauri::State;
use tokio::process::Command;

use crate::db::get_state;
use crate::types::AppState;

use super::cwd_util::resolve_cwd;

fn app_name(target: &str) -> Option<&'static str> {
    match target {
        "vscode" => Some("Visual Studio Code"),
        "cursor" => Some("Cursor"),
        "sublime" => Some("Sublime Text"),
        "zed" => Some("Zed"),
        "antigravity" => Some("Antigravity"),
        "terminal" => Some("Terminal"),
        _ => None,
    }
}

#[tauri::command]
pub(crate) async fn open_workspace_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    target: String,
) -> Result<(), String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    if !cwd.exists() {
        return Err(format!("workspace path does not exist: {}", cwd.display()));
    }

    let mut cmd = Command::new("open");
    if target == "finder" {
        cmd.arg(&cwd);
    } else if let Some(app) = app_name(target.trim()) {
        cmd.args(["-a", app]).arg(&cwd);
    } else {
        return Err(format!("unsupported workspace target: {target}"));
    }

    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("open workspace failed: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("open workspace failed with status: {status}"))
    }
}
