use serde::Serialize;
use tauri::State;
use tokio::io::AsyncReadExt;

use crate::db::get_state;
use crate::fs_context::looks_binary;
use crate::git::{self, GitError, GitStatus};
use crate::types::AppState;

use super::cwd_util::{resolve_cwd, resolve_within_cwd};

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitState {
    NotRepo { cwd: String },
    GitMissing,
    Status(GitStatus),
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
    untracked: bool,
) -> Result<String, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    resolve_within_cwd(&cwd, path.trim_end_matches('/')).await?;
    git::diff(&cwd, &path, vs_head, staged, untracked)
        .await
        .map_err(|e| e.to_string())
}

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[tauri::command]
pub(crate) async fn git_file_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    path: String,
) -> Result<String, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    let canonical = resolve_within_cwd(&cwd, path.trim_end_matches('/')).await?;
    let meta = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("stat failed: {e}"))?;
    if meta.is_dir() {
        return Err("path is a directory".to_string());
    }
    let file = tokio::fs::File::open(&canonical)
        .await
        .map_err(|e| format!("open failed: {e}"))?;
    let mut bytes = Vec::with_capacity((meta.len() as usize).min(MAX_FILE_BYTES + 1));
    let mut reader = file.take((MAX_FILE_BYTES + 1) as u64);
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| format!("read failed: {e}"))?;
    let truncated = bytes.len() > MAX_FILE_BYTES;
    if truncated {
        bytes.truncate(MAX_FILE_BYTES);
    }
    let inspect = bytes.len().min(8000);
    if looks_binary(&bytes[..inspect]) {
        return Ok("(binary file)".to_string());
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[file truncated]\n");
    }
    Ok(text)
}
