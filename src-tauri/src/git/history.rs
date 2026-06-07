use std::path::Path;
use std::process::Stdio;

use git2::{Delta, DiffOptions, Repository, Sort};
use serde::Serialize;

use super::error::GitError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFileEntry {
    pub path: String,
    pub status_letter: String,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author_name: String,
    pub committed_at: i64,
    pub files: Vec<CommitFileEntry>,
    pub additions: u32,
    pub deletions: u32,
}

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

pub fn commit_detail(cwd: &Path, oid: &str) -> Result<CommitDetail, GitError> {
    let repo = Repository::open(cwd)?;
    let oid = oid.trim();
    let oid = git2::Oid::from_str(oid).map_err(|e| GitError::CommandFailed(e.to_string()))?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut files = Vec::new();
    let mut additions = 0u32;
    let mut deletions = 0u32;

    let mut opts = DiffOptions::new();
    let mut diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    diff.find_similar(None)?;

    diff.foreach(
        &mut |delta, _| {
            let status_letter = match delta.status() {
                Delta::Added => "A",
                Delta::Deleted => "D",
                Delta::Modified => "M",
                Delta::Renamed => "R",
                Delta::Copied => "C",
                Delta::Typechange => "T",
                Delta::Ignored
                | Delta::Unmodified
                | Delta::Untracked
                | Delta::Conflicted
                | Delta::Unreadable => "M",
            }
            .to_string();

            let (path, old_path) = match delta.status() {
                Delta::Deleted => {
                    let p = delta
                        .old_file()
                        .path()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (p, None)
                }
                Delta::Renamed | Delta::Copied => {
                    let new_p = delta
                        .new_file()
                        .path()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let old_p = delta
                        .old_file()
                        .path()
                        .map(|p| p.to_string_lossy().into_owned());
                    (new_p, old_p)
                }
                _ => {
                    let p = delta
                        .new_file()
                        .path()
                        .or_else(|| delta.old_file().path())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (p, None)
                }
            };

            if !path.is_empty() {
                files.push(CommitFileEntry {
                    path,
                    status_letter,
                    old_path,
                });
            }
            true
        },
        None,
        None,
        Some(&mut |_delta, _range, line| {
            match line.origin() {
                '+' => additions += 1,
                '-' => deletions += 1,
                _ => {}
            }
            true
        }),
    )?;

    let full = oid.to_string();
    let summary = commit
        .summary()
        .unwrap_or("(no commit message)")
        .to_string();
    let author_name = commit.author().name().unwrap_or("Unknown").to_string();
    let committed_at = commit.time().seconds() * 1000;

    Ok(CommitDetail {
        oid: full.clone(),
        short_oid: full.chars().take(8).collect(),
        summary,
        author_name,
        committed_at,
        files,
        additions,
        deletions,
    })
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

pub async fn commit_file_diff(cwd: &Path, oid: &str, path: &str) -> Result<String, GitError> {
    let oid = oid.trim();
    let path = path.trim();
    if oid.is_empty() {
        return Err(GitError::CommandFailed(
            "commit oid is required".to_string(),
        ));
    }
    if path.is_empty() {
        return Err(GitError::CommandFailed("file path is required".to_string()));
    }

    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args([
            "show",
            "--format=",
            "--no-color",
            "--find-renames",
            oid,
            "--",
            path,
        ])
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
