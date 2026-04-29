use std::path::Path;
use std::process::Stdio;

use git2::{Repository, Sort};
use serde::Serialize;

use super::error::GitError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitEntry {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author_name: String,
    pub committed_at: i64,
}

pub fn list_commits(cwd: &Path, limit: usize) -> Result<Vec<CommitEntry>, GitError> {
    let repo = Repository::open(cwd)?;
    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TIME)?;
    revwalk.push_head()?;

    let mut out = Vec::new();
    for oid_res in revwalk.take(limit) {
        let oid = oid_res?;
        let commit = repo.find_commit(oid)?;
        let full = oid.to_string();
        let short_oid = full.chars().take(8).collect::<String>();
        out.push(CommitEntry {
            oid: full,
            short_oid,
            summary: commit
                .summary()
                .unwrap_or("(no commit message)")
                .to_string(),
            author_name: commit.author().name().unwrap_or("Unknown").to_string(),
            committed_at: commit.time().seconds() * 1000,
        });
    }
    Ok(out)
}

pub async fn commit_diff(cwd: &Path, oid: &str) -> Result<String, GitError> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["show", "--format=", "--no-color", "--find-renames", oid])
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
        return Err(GitError::CommandFailed(stderr.trim().to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
