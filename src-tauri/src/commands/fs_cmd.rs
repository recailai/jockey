use serde::Serialize;
use tauri::State;

use crate::db::get_state;
use crate::fs_context::should_skip_name;
use crate::types::AppState;

use super::cwd_util::{resolve_cwd, resolve_within_cwd};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[tauri::command]
pub(crate) async fn list_dir_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    rel_path: String,
) -> Result<Vec<DirEntry>, String> {
    let state_ref = get_state(&state);
    let sid = app_session_id.as_deref().filter(|s| !s.trim().is_empty());
    let cwd_str = sid
        .and_then(|id| crate::db::app_session::get_app_session_cwd(state_ref, id))
        .ok_or_else(|| "no working directory set for this session".to_string())?;
    let cwd = std::path::PathBuf::from(&cwd_str);
    let trimmed = rel_path.trim_start_matches('/');
    let target = resolve_within_cwd(&cwd, trimmed).await?;

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|e| format!("stat failed: {e}"))?;
    if !meta.is_dir() {
        return Err("path is not a directory".to_string());
    }

    let mut reader = tokio::fs::read_dir(&target)
        .await
        .map_err(|e| format!("read_dir failed: {e}"))?;
    let mut entries: Vec<DirEntry> = Vec::new();
    while let Some(dent) = reader
        .next_entry()
        .await
        .map_err(|e| format!("next_entry failed: {e}"))?
    {
        let name = match dent.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if should_skip_name(&name) {
            continue;
        }
        let is_dir = dent
            .file_type()
            .await
            .map(|t| t.is_dir())
            .unwrap_or(false);
        entries.push(DirEntry { name, is_dir });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}
