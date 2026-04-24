use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use super::error::GitError;

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

pub async fn diff(
    cwd: &Path,
    path: &str,
    vs_head: bool,
    staged: bool,
) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).arg("diff");
    if staged {
        cmd.arg("--cached");
    }
    if vs_head {
        cmd.arg("HEAD");
    }
    cmd.arg("--");
    cmd.arg(path);

    let output = cmd
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

    let mut bytes = output.stdout;
    let truncated = bytes.len() > MAX_DIFF_BYTES;
    if truncated {
        bytes.truncate(MAX_DIFF_BYTES);
    }

    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.contains("Binary files") && text.contains("differ") {
        return Ok("binary file changed".to_string());
    }
    if truncated {
        text.push_str("\n[diff truncated]\n");
    }
    Ok(text)
}
