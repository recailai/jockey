use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use super::super::super::adapter::{acp_log, clip, NativeProtocol};
pub(super) use crate::acp::process::terminate_pid;
use crate::acp::process::AgentProcess;

pub(super) struct NativeProcess {
    process: AgentProcess,
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
        let process = AgentProcess::spawn(binary, args, env, cwd, diagnostic, "native.stderr")?;
        Ok(Self {
            process,
            next_id: 1,
        })
    }

    pub(super) fn pid(&self) -> Option<u32> {
        self.process.pid()
    }

    pub(super) fn is_alive(&mut self) -> bool {
        self.process.is_alive()
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
        let mut stdin = self.process.stdin.lock().await;
        stdin
            .write_all(&frame)
            .await
            .map_err(|error| self.io_error("write", error))?;
        stdin
            .flush()
            .await
            .map_err(|error| self.io_error("flush", error))
    }

    pub(super) fn stdin_handle(
        &self,
    ) -> std::sync::Arc<tokio::sync::Mutex<tokio::process::ChildStdin>> {
        self.process.stdin_handle()
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
            if is_server_request(&message, protocol) {
                self.answer_server_request(protocol, &message, auto_approve)
                    .await?;
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
        protocol: NativeProtocol,
        timeout: Duration,
        auto_approve: bool,
    ) -> Result<Value, String> {
        loop {
            let message = self.next_message(timeout).await?;
            if is_server_request(&message, protocol) {
                self.answer_server_request(protocol, &message, auto_approve)
                    .await?;
                continue;
            }
            return Ok(message);
        }
    }

    /// Read the next frame without interpreting server requests. Callers that can surface an
    /// approval to the user take this and resolve the request themselves; `next_agent_message`
    /// remains the auto-answering path for transports with no interactive surface.
    pub(super) async fn next_frame(&mut self, timeout: Duration) -> Result<Value, String> {
        self.next_message(timeout).await
    }

    pub(super) fn server_request_id(message: &Value) -> Option<Value> {
        if message.get("method").and_then(Value::as_str).is_some() && message.get("id").is_some() {
            message.get("id").cloned()
        } else {
            None
        }
    }

    async fn next_message(&mut self, timeout: Duration) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = tokio::time::timeout(timeout, self.process.stdout.read_line(&mut line))
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
        protocol: NativeProtocol,
        message: &Value,
        auto_approve: bool,
    ) -> Result<(), String> {
        match protocol {
            NativeProtocol::CodexAppServer => {
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
            NativeProtocol::PiRpc => {
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(id) = message.get("id").cloned() else {
                    return Ok(());
                };
                // Fire-and-forget UI updates in Pi extensions: do not send any reply
                if method == "setStatus" || method == "notify" || method == "setWidget" {
                    return Ok(());
                }
                // Interactive UI dialogs in Pi extensions
                if method == "confirm" {
                    return self
                        .send(json!({
                            "type": "extension_ui_response",
                            "id": id,
                            "confirmed": auto_approve
                        }))
                        .await;
                }
                if method == "select" {
                    let first_opt = message
                        .get("options")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.first())
                        .cloned()
                        .unwrap_or(Value::Null);
                    return self
                        .send(json!({
                            "type": "extension_ui_response",
                            "id": id,
                            "value": first_opt
                        }))
                        .await;
                }
                Ok(())
            }
        }
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
        let tail = self.process.stderr_tail();
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", tail.trim())
        }
    }

    pub(super) async fn close(&mut self) {
        self.process.close(Duration::from_secs(2)).await;
    }
}

fn is_server_request(message: &Value, protocol: NativeProtocol) -> bool {
    match protocol {
        NativeProtocol::CodexAppServer => {
            message.get("method").and_then(Value::as_str).is_some() && message.get("id").is_some()
        }
        NativeProtocol::PiRpc => {
            message.get("type").and_then(Value::as_str) == Some("extension_ui_request")
        }
    }
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
