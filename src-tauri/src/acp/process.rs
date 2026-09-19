use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex as AsyncMutex;

use super::adapter::{acp_log, clip};
use super::error::{push_stderr_tail, stderr_tail};

pub(crate) struct AgentProcess {
    pub(crate) child: Child,
    pub(crate) stdin: Arc<AsyncMutex<ChildStdin>>,
    pub(crate) stdout: BufReader<ChildStdout>,
    pub(crate) stderr: Arc<std::sync::Mutex<String>>,
    pub(crate) stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl AgentProcess {
    pub(crate) fn spawn(
        binary: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
        diagnostic: &str,
        log_event: &'static str,
    ) -> Result<Self, String> {
        let mut command = Command::new(binary);
        command
            .args(args)
            .envs(
                env.iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start {diagnostic}: {error}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("{diagnostic} stdin pipe unavailable"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("{diagnostic} stdout pipe unavailable"))?;

        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_task = child.stderr.take().map(|stream| {
            let stderr = stderr.clone();
            let diagnostic = diagnostic.to_string();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            push_stderr_tail(&stderr, &line);
                            if !line.trim().is_empty() {
                                acp_log(
                                    log_event,
                                    json!({ "runtime": diagnostic, "line": clip(line.trim(), 360) }),
                                );
                            }
                        }
                    }
                }
            })
        });

        Ok(Self {
            child,
            stdin: Arc::new(AsyncMutex::new(stdin)),
            stdout: BufReader::new(stdout),
            stderr,
            stderr_task,
        })
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub(crate) fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub(crate) fn stderr_tail(&self) -> String {
        stderr_tail(&self.stderr)
    }

    pub(crate) fn stdin_handle(&self) -> Arc<AsyncMutex<ChildStdin>> {
        self.stdin.clone()
    }

    pub(crate) async fn close(&mut self, timeout: Duration) {
        let mut stdin = self.stdin.lock().await;
        let _ = stdin.shutdown().await;
        drop(stdin);
        if let Some(pid) = self.pid() {
            terminate_pid(pid);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill().await;
        }
        let _ = tokio::time::timeout(timeout, self.child.wait()).await;
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await;
        }
    }
}

pub(crate) fn terminate_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let pgid = -(pid as i32);
        let _ = libc::kill(pgid, libc::SIGTERM);
        let _ = libc::kill(pgid, libc::SIGKILL);
        let _ = libc::kill(pid as i32, libc::SIGTERM);
        let _ = libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}
