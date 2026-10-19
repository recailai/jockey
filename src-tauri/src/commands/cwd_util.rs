use std::path::{Path, PathBuf};

use crate::db::app_session::get_app_session_cwd;
use crate::resolve_chat_cwd;
use crate::types::AppState;

/// Working directories of draft sessions — sessions that exist only in a view until their first
/// send creates the row. Without this, every session-scoped command (open in IDE, git panel,
/// file tree) resolved a draft to `$HOME` instead of its project.
fn draft_cwds() -> &'static dashmap::DashMap<String, String> {
    static DRAFTS: std::sync::OnceLock<dashmap::DashMap<String, String>> =
        std::sync::OnceLock::new();
    DRAFTS.get_or_init(dashmap::DashMap::new)
}

/// A view tells the backend where its draft session lives.
#[tauri::command]
pub(crate) fn register_draft_session_cwd(app_session_id: String, cwd: String) {
    if !app_session_id.trim().is_empty() && !cwd.trim().is_empty() {
        draft_cwds().insert(app_session_id, cwd);
    }
}

/// Session working directory: the session row's `cwd`, else its project's root, else the
/// draft registration, else the default chat directory.
pub(crate) fn resolve_cwd(state: &AppState, app_session_id: Option<&str>) -> PathBuf {
    let sid = app_session_id.filter(|s| !s.trim().is_empty());
    let cwd = sid
        .and_then(|sid| get_app_session_cwd(state, sid))
        .filter(|c| !c.trim().is_empty())
        .or_else(|| {
            let pid = crate::db::app_session::get_app_session_project_id(state, sid?)?;
            crate::db::project::get_project_internal(state, &pid)
                .ok()
                .flatten()
                .map(|p| p.root_path)
        })
        .or_else(|| sid.and_then(|sid| draft_cwds().get(sid).map(|c| c.clone())))
        .unwrap_or_else(resolve_chat_cwd);
    PathBuf::from(cwd)
}

pub(crate) async fn resolve_within_cwd(cwd: &Path, rel: &str) -> Result<PathBuf, String> {
    let trimmed = rel.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(cwd.to_path_buf());
    }
    if trimmed.contains("..") || trimmed.starts_with('/') {
        let target = tokio::fs::canonicalize(cwd.join(trimmed))
            .await
            .map_err(|e| format!("cannot resolve path: {e}"))?;
        let cwd_canonical = tokio::fs::canonicalize(cwd)
            .await
            .map_err(|e| format!("cannot resolve cwd: {e}"))?;
        if !target.starts_with(&cwd_canonical) {
            return Err("path escapes session cwd".to_string());
        }
        return Ok(target);
    }
    let target = cwd.join(trimmed);
    if !target.starts_with(cwd) {
        return Err("path escapes session cwd".to_string());
    }
    Ok(target)
}
