use std::path::Path;
use std::process::{Command, Stdio};

use git2::{
    BranchType, Commit, ObjectType, Repository,
    Signature, Status, StatusOptions,
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

pub fn stage_path(cwd: &Path, rel_path: &str) -> Result<(), GitError> {
    let repo = Repository::open(cwd)?;
    let mut index = repo.index()?;
    let path = Path::new(rel_path);
    let status = repo.status_file(path).unwrap_or(Status::CURRENT);
    if status.contains(Status::WT_DELETED) {
        index.remove_path(path)?;
    } else {
        index.add_path(path)?;
    }
    index.write()?;
    Ok(())
}

pub fn unstage_path(cwd: &Path, rel_path: &str) -> Result<(), GitError> {
    let repo = Repository::open(cwd)?;
    let path = Path::new(rel_path);

    if let Ok(head_ref) = repo.head() {
        let head_obj = head_ref.peel(ObjectType::Commit)?;
        repo.reset_default(Some(&head_obj), [path])?;
    } else {
        let mut index = repo.index()?;
        index.remove_path(path)?;
        index.write()?;
    }
    Ok(())
}

pub fn fetch(cwd: &Path) -> Result<(), GitError> {
    run_git_sync(cwd, &["fetch", "--prune", "origin"])
}

pub fn pull(cwd: &Path) -> Result<(), GitError> {
    run_git_sync(cwd, &["pull", "--ff-only"])
}

fn run_git_sync(cwd: &Path, args: &[&str]) -> Result<(), GitError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| GitError::CommandFailed(format!("git {} failed: {e}", args.join(" "))))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(GitError::CommandFailed(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        }))
    }
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
    let branch = repo.find_branch(&branch_name, BranchType::Local)?;
    let (remote_name, remote_branch_name, set_upstream) =
        resolve_push_target(&branch, &branch_name)?;

    let mut args = vec!["push"];
    if set_upstream {
        args.push("-u");
    }
    args.push(&remote_name);
    let refspec = format!("{branch_name}:{remote_branch_name}");
    args.push(&refspec);

    run_git_sync(cwd, &args)?;
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

#[cfg(test)]
mod tests {
    use super::{stage_path, unstage_path};
    use git2::{Repository, Status, StatusOptions};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn init_repo() -> Result<(TempDir, Repository), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let repo = Repository::init(dir.path())?;
        Ok((dir, repo))
    }

    fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        fs::write(path, contents)?;
        Ok(())
    }

    fn commit_all(repo: &Repository, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        let sig = git2::Signature::now("Jockey Test", "jockey@example.com")?;
        let mut index = repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;
        let parents = match repo.head() {
            Ok(head) => vec![head.peel_to_commit()?],
            Err(_) => Vec::new(),
        };
        let parent_refs: Vec<_> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)?;
        Ok(())
    }

    fn status_for(repo: &Repository, rel_path: &str) -> Result<Status, Box<dyn std::error::Error>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true)
            .pathspec(rel_path);
        let statuses = repo.statuses(Some(&mut opts))?;
        Ok(statuses
            .iter()
            .find(|entry| entry.path() == Some(rel_path))
            .map(|entry| entry.status())
            .unwrap_or(Status::CURRENT))
    }

    #[test]
    fn stage_path_handles_deleted_files() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, repo) = init_repo()?;
        let file_path = repo.path().parent().unwrap().join("tracked.txt");
        write_file(&file_path, "hello\n")?;
        commit_all(&repo, "initial")?;

        fs::remove_file(&file_path)?;
        stage_path(repo.path().parent().unwrap(), "tracked.txt")
            .map_err(|e| format!("stage deleted file failed: {e}"))?;

        let status = status_for(&repo, "tracked.txt")?;
        assert!(status.contains(Status::INDEX_DELETED));
        assert!(!status.contains(Status::WT_DELETED));
        Ok(())
    }

    #[test]
    fn unstage_path_restores_tracked_file_to_head() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, repo) = init_repo()?;
        let file_path = repo.path().parent().unwrap().join("tracked.txt");
        write_file(&file_path, "before\n")?;
        commit_all(&repo, "initial")?;

        write_file(&file_path, "after\n")?;
        stage_path(repo.path().parent().unwrap(), "tracked.txt")
            .map_err(|e| format!("stage tracked file failed: {e}"))?;
        unstage_path(repo.path().parent().unwrap(), "tracked.txt")
            .map_err(|e| format!("unstage tracked file failed: {e}"))?;

        let status = status_for(&repo, "tracked.txt")?;
        assert!(status.contains(Status::WT_MODIFIED));
        assert!(
            !status.intersects(Status::INDEX_MODIFIED | Status::INDEX_DELETED | Status::INDEX_NEW)
        );
        Ok(())
    }

    #[test]
    fn unstage_path_removes_new_file_from_index() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, repo) = init_repo()?;
        let file_path = repo.path().parent().unwrap().join("new.txt");
        write_file(&file_path, "hello\n")?;

        stage_path(repo.path().parent().unwrap(), "new.txt")
            .map_err(|e| format!("stage new file failed: {e}"))?;
        unstage_path(repo.path().parent().unwrap(), "new.txt")
            .map_err(|e| format!("unstage new file failed: {e}"))?;

        let status = status_for(&repo, "new.txt")?;
        assert!(status.contains(Status::WT_NEW));
        assert!(
            !status.intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED)
        );
        Ok(())
    }
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


