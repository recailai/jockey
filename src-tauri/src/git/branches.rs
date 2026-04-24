use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

use super::error::GitError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

pub async fn list_branches(cwd: &Path) -> Result<Vec<BranchInfo>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["branch", "--format=%(refname:short)\t%(upstream:short)\t%(HEAD)", "-a"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitNotFound
            } else {
                GitError::CommandFailed(e.to_string())
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not a git repository") {
            return Err(GitError::NotARepo);
        }
        return Err(GitError::CommandFailed(stderr.into_owned()));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut branches: Vec<BranchInfo> = Vec::new();

    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts[0].trim();
        if name.is_empty() {
            continue;
        }
        if name.starts_with("remotes/") {
            continue;
        }
        let upstream = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string());
        let is_current = parts.get(2).map(|s| s.trim() == "*").unwrap_or(false);
        branches.push(BranchInfo {
            name: name.to_string(),
            is_current,
            upstream,
        });
    }

    Ok(branches)
}

pub async fn checkout(cwd: &Path, branch: &str) -> Result<(), GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["checkout", branch])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitNotFound
            } else {
                GitError::CommandFailed(e.to_string())
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(stderr.into_owned()));
    }

    Ok(())
}
