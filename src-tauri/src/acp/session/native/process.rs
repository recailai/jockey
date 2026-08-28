use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::super::super::adapter::{acp_log, clip, NativeProtocol};
use super::super::super::error::{push_stderr_tail, stderr_tail};

pub(super) struct NativeProcess {
    pub(super) child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<std::sync::Mutex<String>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    next_id: u64,
}

impl NativeProcess {
    pub(super) fn spawn(
        binary: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
        diagnostic: &str,
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
                                    "native.stderr",
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
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
            stderr_task,
            next_id: 1,
        })
    }

    pub(super) fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub(super) fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub(super) async fn send(&mut self, frame: Value) -> Result<(), String> {
        let mut frame = serde_json::to_vec(&frame)
            .map_err(|error| format!("native RPC frame serialization failed: {error}"))?;
        frame.push(b'\n');
        self.stdin
            .write_all(&frame)
            .await
            .map_err(|error| self.io_error("write", error))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| self.io_error("flush", error))
    }

    pub(super) async fn request(
        &mut self,
        protocol: NativeProtocol,
        method: &str,
        params: Value,
        timeout: Duration,
        auto_approve: bool,
    ) -> Result<(Value, Vec<Value>), String> {
        let request_id = self.next_request_id();
        let id = match protocol {
            NativeProtocol::CodexAppServer => json!(request_id),
            NativeProtocol::PiRpc => json!(format!("req_{request_id}")),
        };
        let frame = if protocol == NativeProtocol::PiRpc {
            let mut frame = json!({ "id": id.clone(), "type": method });
            if let Some(object) = params.as_object() {
                for (key, value) in object {
                    frame[key.as_str()] = value.clone();
                }
            }
            frame
        } else {
            json!({ "id": id.clone(), "method": method, "params": params })
        };
        self.send(frame).await?;
        let mut side_messages = Vec::new();
        loop {
            let message = self.next_message(timeout).await?;
            if is_server_request(&message) {
                self.answer_server_request(&message, auto_approve).await?;
                continue;
            }
            if message.get("id") == Some(&id) && is_response(&message, protocol) {
                if let Some(error) = message.get("error") {
                    return Err(format_rpc_error(method, error));
                }
                if protocol == NativeProtocol::PiRpc
                    && message.get("success").and_then(Value::as_bool) == Some(false)
                {
                    return Err(message
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Pi RPC request failed")
                        .to_string());
                }
                let result = message
                    .get("result")
                    .or_else(|| message.get("data"))
                    .cloned()
                    .unwrap_or(Value::Null);
                return Ok((result, side_messages));
            }
            side_messages.push(message);
        }
    }

    pub(super) async fn next_agent_message(
        &mut self,
        timeout: Duration,
        auto_approve: bool,
    ) -> Result<Value, String> {
        loop {
            let message = self.next_message(timeout).await?;
            if is_server_request(&message) {
                self.answer_server_request(&message, auto_approve).await?;
                continue;
            }
            return Ok(message);
        }
    }

    async fn next_message(&mut self, timeout: Duration) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = tokio::time::timeout(timeout, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| self.timeout_error())?
                .map_err(|error| self.io_error("read", error))?;
            if read == 0 {
                return Err(self.exit_error());
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(message) => return Ok(message),
                Err(error) => {
                    acp_log(
                        "native.stdout.unparsed",
                        json!({ "line": clip(trimmed, 360), "error": error.to_string() }),
                    );
                }
            }
        }
    }

    async fn answer_server_request(
        &mut self,
        message: &Value,
        auto_approve: bool,
    ) -> Result<(), String> {
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let result = if method.contains("approval") || method.contains("permission") {
            json!({ "decision": if auto_approve { "accept" } else { "decline" } })
        } else {
            json!({})
        };
        self.send(json!({ "id": id, "result": result })).await
    }

    fn io_error(&self, operation: &str, error: std::io::Error) -> String {
        format!(
            "native RPC {operation} failed: {error}{}",
            self.stderr_suffix()
        )
    }

    fn timeout_error(&self) -> String {
        format!("native RPC timed out{}", self.stderr_suffix())
    }

    fn exit_error(&self) -> String {
        format!("native RPC process exited{}", self.stderr_suffix())
    }

    fn stderr_suffix(&self) -> String {
        let tail = stderr_tail(&self.stderr);
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", tail.trim())
        }
    }

    pub(super) async fn close(&mut self) {
        let _ = self.stdin.shutdown().await;
        if let Some(pid) = self.child.id() {
            terminate_pid(pid);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await;
        }
    }
}

fn is_server_request(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str).is_some() && message.get("id").is_some()
}

fn is_response(message: &Value, protocol: NativeProtocol) -> bool {
    match protocol {
        NativeProtocol::CodexAppServer => {
            message.get("result").is_some() || message.get("error").is_some()
        }
        NativeProtocol::PiRpc => message.get("type").and_then(Value::as_str) == Some("response"),
    }
}

fn format_rpc_error(method: &str, error: &Value) -> String {
    let detail = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());
    format!("native RPC {method} failed: {detail}")
}

pub(super) fn transport_name(protocol: NativeProtocol) -> &'static str {
    match protocol {
        NativeProtocol::CodexAppServer => "codex-app-server",
        NativeProtocol::PiRpc => "pi-rpc",
    }
}

#[cfg(unix)]
pub(super) fn terminate_pid(pid: u32) {
    unsafe {
        let pgid = -(pid as i32);
        let _ = libc::kill(pgid, libc::SIGTERM);
        let _ = libc::kill(pgid, libc::SIGKILL);
        let _ = libc::kill(pid as i32, libc::SIGTERM);
        let _ = libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
pub(super) fn terminate_pid(_pid: u32) {}
