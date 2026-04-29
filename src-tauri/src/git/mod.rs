pub(crate) mod branches;
pub(crate) mod diff;
pub(crate) mod error;
pub(crate) mod forge;
pub(crate) mod history;
pub(crate) mod remote;
pub(crate) mod status;
pub(crate) mod write;

use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

pub(crate) use branches::{checkout, list_branches, BranchInfo};
pub(crate) use diff::diff;
pub(crate) use error::GitError;
pub(crate) use forge::{create_pull_request, CreatedPullRequest};
pub(crate) use history::{commit_diff, list_commits, CommitEntry};
pub(crate) use remote::{read_origin_info, urlencoding_minimal};
pub(crate) use status::{status, GitStatus};
pub(crate) use write::{commit, push};

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub(crate) fn set_app_handle(handle: AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

pub(crate) fn notify_changed(path: &std::path::Path) {
    if let Some(handle) = APP_HANDLE.get() {
        let _ = handle.emit(
            "git/changed",
            serde_json::json!({ "path": path.to_string_lossy() }),
        );
    }
}
