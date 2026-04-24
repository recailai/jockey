use serde::Serialize;
use tauri::State;
use tokio::io::AsyncReadExt;

use crate::db::get_state;
use crate::fs_context::looks_binary;
use crate::git::{self, BranchInfo, GitError, GitStatus};
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

#[tauri::command]
pub(crate) async fn git_list_branches_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
) -> Result<Vec<BranchInfo>, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    git::list_branches(&cwd).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn git_checkout_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    branch: String,
) -> Result<(), String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    git::checkout(&cwd, &branch).await.map_err(|e| e.to_string())?;
    let head_path = cwd.join(".git").join("HEAD");
    git::notify_changed(&head_path);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteInfo {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub web_url: String,
    pub branch_url: Option<String>,
    pub pr_url: Option<String>,
    pub compare_url: Option<String>,
}

fn parse_remote_url(raw: &str) -> Option<(String, String, String)> {
    let url = raw.trim();
    if url.is_empty() {
        return None;
    }

    if let Some(rest) = url.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return split_owner_repo(host, path);
        }
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        if let Some((host, path)) = rest.split_once('/') {
            let host = host.split('@').next_back().unwrap_or(host);
            let host = host.split(':').next().unwrap_or(host);
            return split_owner_repo(host, path);
        }
    }
    for prefix in ["https://", "http://", "git://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            let rest = rest.split('@').next_back().unwrap_or(rest);
            if let Some((host, path)) = rest.split_once('/') {
                let host = host.split(':').next().unwrap_or(host);
                return split_owner_repo(host, path);
            }
        }
    }
    None
}

fn split_owner_repo(host: &str, path: &str) -> Option<(String, String, String)> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((host.to_string(), owner, repo))
}

async fn read_remote_origin(cwd: &std::path::Path) -> Option<String> {
    use std::process::Stdio;
    let output = tokio::process::Command::new("git")
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn build_remote_info(
    host: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
) -> GitRemoteInfo {
    let web_url = format!("https://{host}/{owner}/{repo}");
    let (branch_url, pr_url, compare_url) = match branch {
        Some(b) if !b.is_empty() => {
            let enc = urlencoding_minimal(b);
            (
                Some(format!("{web_url}/tree/{enc}")),
                Some(format!("{web_url}/pull/{enc}")),
                Some(format!("{web_url}/compare/{enc}?expand=1")),
            )
        }
        _ => (None, None, None),
    };
    GitRemoteInfo {
        host: host.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        web_url,
        branch_url,
        pr_url,
        compare_url,
    }
}

fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[tauri::command]
pub(crate) async fn git_remote_info_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
) -> Result<Option<GitRemoteInfo>, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    let Some(raw) = read_remote_origin(&cwd).await else {
        return Ok(None);
    };
    let Some((host, owner, repo)) = parse_remote_url(&raw) else {
        return Ok(None);
    };
    let branch = match git::status(&cwd).await {
        Ok(st) => st.branch,
        Err(_) => None,
    };
    Ok(Some(build_remote_info(&host, &owner, &repo, branch.as_deref())))
}

#[tauri::command]
pub(crate) async fn git_pr_url_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
) -> Result<Option<String>, String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    let Some(raw) = read_remote_origin(&cwd).await else {
        return Ok(None);
    };
    let Some((host, owner, repo)) = parse_remote_url(&raw) else {
        return Ok(None);
    };
    let branch = match git::status(&cwd).await {
        Ok(st) => st.branch,
        Err(_) => None,
    };
    let info = build_remote_info(&host, &owner, &repo, branch.as_deref());
    Ok(info.pr_url.or(Some(info.web_url)))
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
