use std::path::{Path, PathBuf};

use crate::db::app_session::get_app_session_cwd;
use crate::resolve_chat_cwd;
use crate::types::AppState;

pub(crate) fn resolve_cwd(state: &AppState, app_session_id: Option<&str>) -> PathBuf {
    let cwd = app_session_id
        .filter(|s| !s.trim().is_empty())
        .and_then(|sid| get_app_session_cwd(state, sid))
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
