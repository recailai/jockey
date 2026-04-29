use std::path::Path;
use std::process::{Command, Stdio};

use git2::{
    BranchType, Commit, Cred, CredentialType, PushOptions, RemoteCallbacks, Repository, Signature,
    Status, StatusOptions,
};

use super::error::GitError;

pub fn commit(cwd: &Path, message: &str, include_unstaged: bool) -> Result<String, GitError> {
    let repo = Repository::open(cwd)?;
    let mut index = repo.index()?;

    if include_unstaged {
        stage_workdir_changes(&repo, &mut index)?;
        index.write()?;
    }

    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    let parent_commit = read_parent_commit(&repo)?;
    if tree_matches_head(&tree, parent_commit.as_ref()) {
        return Err(GitError::CommandFailed("no changes to commit".to_string()));
    }

    let signature = repo.signature().or_else(|_| fallback_signature())?;
    let parent_refs: Vec<&Commit<'_>> = parent_commit.iter().collect();
    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )?;
    Ok(oid.to_string())
}

pub fn push(cwd: &Path) -> Result<(), GitError> {
    let repo = Repository::open(cwd)?;
    let head = repo.head()?;
    if !head.is_branch() {
        return Err(GitError::CommandFailed(
            "cannot push from a detached HEAD".to_string(),
        ));
    }
    let branch_name = head
        .shorthand()
        .ok_or_else(|| GitError::CommandFailed("current branch has no shorthand name".to_string()))?
        .to_string();
    let mut branch = repo.find_branch(&branch_name, BranchType::Local)?;
    let (remote_name, remote_branch_name, set_upstream) =
        resolve_push_target(&branch, &branch_name)?;

    let mut remote = repo.find_remote(&remote_name)?;
    let callbacks = build_remote_callbacks(&repo, remote.url());
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);

    let refspec = format!("refs/heads/{branch_name}:refs/heads/{remote_branch_name}");
    remote.push(&[refspec.as_str()], Some(&mut options))?;
    if set_upstream {
        branch.set_upstream(Some(&format!("{remote_name}/{remote_branch_name}")))?;
    }
    Ok(())
}

fn stage_workdir_changes(repo: &Repository, index: &mut git2::Index) -> Result<(), GitError> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut options))?;
    for entry in statuses.iter() {
        let Some(path) = entry.path() else {
            continue;
        };
        let status = entry.status();
        if status.is_conflicted() {
            return Err(GitError::CommandFailed(
                "cannot commit while the repository has merge conflicts".to_string(),
            ));
        }
        if status.contains(Status::WT_DELETED) {
            let _ = index.remove_path(Path::new(path));
        }
        if status.intersects(
            Status::WT_NEW | Status::WT_MODIFIED | Status::WT_RENAMED | Status::WT_TYPECHANGE,
        ) {
            index.add_path(Path::new(path))?;
        }
    }
    Ok(())
}

fn read_parent_commit(repo: &Repository) -> Result<Option<Commit<'_>>, GitError> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let target = match head.target() {
        Some(target) => target,
        None => return Ok(None),
    };
    Ok(Some(repo.find_commit(target)?))
}

fn tree_matches_head(tree: &git2::Tree<'_>, head: Option<&Commit<'_>>) -> bool {
    match head {
        Some(commit) => commit.tree_id() == tree.id(),
        None => tree.iter().next().is_none(),
    }
}

fn fallback_signature() -> Result<Signature<'static>, GitError> {
    let name = std::env::var("GIT_AUTHOR_NAME")
        .or_else(|_| std::env::var("GIT_COMMITTER_NAME"))
        .map_err(|_| {
            GitError::CommandFailed(
                "git user.name is not configured and no GIT_AUTHOR_NAME/GIT_COMMITTER_NAME environment variable was found".to_string(),
            )
        })?;
    let email = std::env::var("GIT_AUTHOR_EMAIL")
        .or_else(|_| std::env::var("GIT_COMMITTER_EMAIL"))
        .map_err(|_| {
            GitError::CommandFailed(
                "git user.email is not configured and no GIT_AUTHOR_EMAIL/GIT_COMMITTER_EMAIL environment variable was found".to_string(),
            )
        })?;
    Signature::now(&name, &email).map_err(Into::into)
}

fn resolve_push_target(
    branch: &git2::Branch<'_>,
    local_branch_name: &str,
) -> Result<(String, String, bool), GitError> {
    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        Err(err) if err.code() == git2::ErrorCode::NotFound => {
            return Ok(("origin".to_string(), local_branch_name.to_string(), true))
        }
        Err(err) => return Err(err.into()),
    };

    let shorthand = upstream
        .name()?
        .or_else(|| upstream.get().shorthand())
        .ok_or_else(|| GitError::CommandFailed("upstream branch has no name".to_string()))?;

    let trimmed = shorthand
        .trim_start_matches("refs/remotes/")
        .trim_start_matches("refs/heads/");
    let (remote_name, remote_branch_name) = trimmed
        .split_once('/')
        .ok_or_else(|| GitError::CommandFailed("cannot parse upstream branch".to_string()))?;
    Ok((
        remote_name.to_string(),
        remote_branch_name.to_string(),
        false,
    ))
}

fn build_remote_callbacks(repo: &Repository, remote_url: Option<&str>) -> RemoteCallbacks<'static> {
    let config = repo.config().ok();
    let remote_url = remote_url.unwrap_or("").to_string();
    let token = env_http_token(&remote_url);

    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |url, username_from_url, allowed| {
        if allowed.contains(CredentialType::USERNAME) {
            return Cred::username(username_from_url.unwrap_or("git"));
        }

        if allowed.contains(CredentialType::SSH_KEY) {
            let username = username_from_url.unwrap_or("git");
            if let Ok(cred) = Cred::ssh_key_from_agent(username) {
                return Ok(cred);
            }
            if let Some(cred) = default_ssh_key_credential(username) {
                return Ok(cred);
            }
        }

        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
            if let Some(config) = config.as_ref() {
                if let Ok(cred) = Cred::credential_helper(config, url, username_from_url) {
                    return Ok(cred);
                }
            }
            if let Some((username, password)) = git_credential_fill(url, username_from_url) {
                return Cred::userpass_plaintext(&username, &password);
            }
            if let Some(token) = token.as_ref() {
                let username = username_from_url.unwrap_or("x-access-token");
                return Cred::userpass_plaintext(username, token);
            }
            if let (Ok(username), Ok(password)) = (
                std::env::var("GIT_HTTP_USERNAME"),
                std::env::var("GIT_HTTP_PASSWORD"),
            ) {
                return Cred::userpass_plaintext(&username, &password);
            }
        }

        if allowed.contains(CredentialType::DEFAULT) {
            return Cred::default();
        }

        if allowed.contains(CredentialType::SSH_KEY) {
            return Err(git2::Error::from_str(
                "remote authentication required but no usable SSH key was found",
            ));
        }

        Err(git2::Error::from_str(
            "remote authentication required but no supported credentials were available",
        ))
    });
    callbacks.push_update_reference(|reference, status| {
        if let Some(status) = status {
            Err(git2::Error::from_str(&format!(
                "push failed for {reference}: {status}"
            )))
        } else {
            Ok(())
        }
    });
    callbacks
}

fn env_http_token(remote_url: &str) -> Option<String> {
    let lower = remote_url.to_ascii_lowercase();
    if lower.contains("github") {
        return std::env::var("GH_TOKEN")
            .ok()
            .or_else(|| std::env::var("GITHUB_TOKEN").ok());
    }
    if lower.contains("gitlab.com") {
        return std::env::var("GITLAB_TOKEN").ok();
    }
    if lower.contains("bitbucket.org") {
        return std::env::var("BITBUCKET_TOKEN").ok();
    }
    None
}

fn default_ssh_key_credential(username: &str) -> Option<Cred> {
    let home = dirs::home_dir()?;
    for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
        let private_key = home.join(".ssh").join(name);
        if !private_key.exists() {
            continue;
        }
        if let Ok(cred) = Cred::ssh_key(username, None, &private_key, None) {
            return Some(cred);
        }
    }
    None
}

fn git_credential_fill(url: &str, username: Option<&str>) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let mut input = format!("protocol={}\nhost={host}\n", parsed.scheme());
    let path = parsed.path().trim_start_matches('/');
    if !path.is_empty() {
        input.push_str(&format!("path={path}\n"));
    }
    if let Some(username) = username.filter(|value| !value.is_empty()) {
        input.push_str(&format!("username={username}\n"));
    }
    input.push('\n');

    let output = Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(input.as_bytes());
            }
            child.wait_with_output().ok()
        })?;

    if !output.status.success() {
        return None;
    }

    let mut found_username: Option<String> = None;
    let mut found_password: Option<String> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix("username=") {
            found_username = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("password=") {
            found_password = Some(value.to_string());
        }
    }
    match (found_username, found_password) {
        (Some(username), Some(password)) => Some((username, password)),
        _ => None,
    }
}
