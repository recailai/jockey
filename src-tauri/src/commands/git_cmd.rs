use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::db::app_session::get_app_session_cwd;
use crate::db::get_state;
use crate::git::{self, GitError, GitStatus};
use crate::resolve_chat_cwd;
use crate::types::AppState;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitState {
    NotRepo { cwd: String },
    GitMissing,
    Status(GitStatus),
}

fn resolve_cwd(state: &AppState, app_session_id: Option<&str>) -> PathBuf {
    let cwd = app_session_id
        .filter(|s| !s.trim().is_empty())
        .and_then(|sid| get_app_session_cwd(state, sid))
        .unwrap_or_else(resolve_chat_cwd);
    PathBuf::from(cwd)
}

#[tauri::command]
pub(crate) async fn git_status_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
) -> Result<GitState, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    match git::status(&cwd).await {
        Ok(status) => Ok(GitState::Status(status)),
        Err(GitError::NotARepo) => Ok(GitState::NotRepo {
            cwd: cwd.to_string_lossy().into_owned(),
        }),
        Err(GitError::GitNotFound) => Ok(GitState::GitMissing),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub(crate) async fn git_diff_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    path: String,
    vs_head: bool,
    staged: bool,
) -> Result<String, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    git::diff(&cwd, &path, vs_head, staged)
        .await
        .map_err(|e| e.to_string())
}
