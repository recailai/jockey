use std::path::Path;

use git2::Repository;

use super::error::GitError;

#[derive(Debug, Clone)]
pub struct RemoteRepoInfo {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

pub fn read_origin_info(cwd: &Path) -> Result<Option<RemoteRepoInfo>, GitError> {
    let repo = Repository::open(cwd)?;
    let remote = match repo.find_remote("origin") {
        Ok(remote) => remote,
        Err(err) if err.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let Some(url) = remote.url() else {
        return Ok(None);
    };
    Ok(parse_remote_url(url))
}

pub fn parse_remote_url(raw: &str) -> Option<RemoteRepoInfo> {
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

fn split_owner_repo(host: &str, path: &str) -> Option<RemoteRepoInfo> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(RemoteRepoInfo {
        host: host.to_string(),
        owner,
        repo,
    })
}

pub fn current_branch_name(cwd: &Path) -> Result<Option<String>, GitError> {
    let repo = Repository::open(cwd)?;
    let head = repo.head()?;
    if !head.is_branch() {
        return Ok(None);
    }
    Ok(head.shorthand().map(|name| name.to_string()))
}

pub fn current_head_summary(cwd: &Path) -> Result<Option<String>, GitError> {
    let repo = Repository::open(cwd)?;
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let commit = head.peel_to_commit()?;
    Ok(commit.summary().map(|s| s.to_string()))
}

pub fn urlencoding_minimal(s: &str) -> String {
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
