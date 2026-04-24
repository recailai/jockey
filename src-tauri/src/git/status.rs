use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

use super::error::GitError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub status_letter: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub detached: bool,
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub untracked: Vec<FileEntry>,
}

pub async fn status(cwd: &Path) -> Result<GitStatus, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain=v2", "--branch", "-uall", "-z"])
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

    parse_porcelain_v2(&output.stdout)
}

fn parse_porcelain_v2(buf: &[u8]) -> Result<GitStatus, GitError> {
    let mut status = GitStatus {
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        detached: false,
        staged: Vec::new(),
        unstaged: Vec::new(),
        untracked: Vec::new(),
    };

    let mut iter = buf.split(|b| *b == 0).peekable();
    while let Some(record) = iter.next() {
        if record.is_empty() {
            continue;
        }
        let line = match std::str::from_utf8(record) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Some(rest) = line.strip_prefix("# ") {
            apply_header(rest, &mut status);
            continue;
        }

        let mut parts = line.splitn(2, ' ');
        let kind = parts.next().unwrap_or("");
        let body = parts.next().unwrap_or("");

        match kind {
            "1" => parse_ordinary(body, &mut status),
            "2" => {
                parse_renamed(body, &mut status);
                let _ = iter.next();
            }
            "u" => parse_unmerged(body, &mut status),
            "?" => status.untracked.push(FileEntry {
                path: body.to_string(),
                status_letter: "??".to_string(),
            }),
            _ => {}
        }
    }

    Ok(status)
}

fn apply_header(rest: &str, status: &mut GitStatus) {
    let mut tokens = rest.split_whitespace();
    let key = tokens.next().unwrap_or("");
    match key {
        "branch.head" => {
            let value = tokens.collect::<Vec<_>>().join(" ");
            if value == "(detached)" {
                status.detached = true;
            } else if !value.is_empty() {
                status.branch = Some(value);
            }
        }
        "branch.upstream" => {
            let value = tokens.collect::<Vec<_>>().join(" ");
            if !value.is_empty() {
                status.upstream = Some(value);
            }
        }
        "branch.ab" => {
            let ahead = tokens.next().unwrap_or("");
            let behind = tokens.next().unwrap_or("");
            status.ahead = ahead.trim_start_matches('+').parse().unwrap_or(0);
            status.behind = behind.trim_start_matches('-').parse().unwrap_or(0);
        }
        _ => {}
    }
}

fn skip_fields(body: &str, n: usize) -> Option<&str> {
    let mut rest = body;
    for _ in 0..n {
        let idx = rest.find(' ')?;
        rest = &rest[idx + 1..];
    }
    Some(rest)
}

fn parse_ordinary(body: &str, status: &mut GitStatus) {
    let xy = body.split_whitespace().next().unwrap_or("");
    if xy.len() < 2 {
        return;
    }
    let staged_letter = &xy[..1];
    let unstaged_letter = &xy[1..2];
    let Some(path) = skip_fields(body, 7) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    push_if_changed(&mut status.staged, staged_letter, path);
    push_if_changed(&mut status.unstaged, unstaged_letter, path);
}

fn parse_renamed(body: &str, status: &mut GitStatus) {
    let xy = body.split_whitespace().next().unwrap_or("");
    if xy.len() < 2 {
        return;
    }
    let staged_letter = &xy[..1];
    let unstaged_letter = &xy[1..2];
    let Some(path) = skip_fields(body, 8) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    push_if_changed(&mut status.staged, staged_letter, path);
    push_if_changed(&mut status.unstaged, unstaged_letter, path);
}

fn parse_unmerged(body: &str, status: &mut GitStatus) {
    let xy = body.split_whitespace().next().unwrap_or("");
    if xy.is_empty() {
        return;
    }
    let Some(path) = skip_fields(body, 9) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    status.unstaged.push(FileEntry {
        path: path.to_string(),
        status_letter: "U".to_string(),
    });
}

fn push_if_changed(target: &mut Vec<FileEntry>, letter: &str, path: &str) {
    if letter == "." || letter.is_empty() {
        return;
    }
    target.push(FileEntry {
        path: path.to_string(),
        status_letter: letter.to_string(),
    });
}
