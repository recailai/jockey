use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

use super::super::adapter::{
    acp_log, clip, friendly_error_message, AdapterTransport, HeadlessProtocol,
};
use super::super::error::{push_stderr_tail, stderr_tail, AcpErrorCode};
use super::super::worker::{register_child_pid, unregister_child_pid, AcpEvent, AcpPromptResult};
use super::execute::{AcpDeltaPayload, AcpStreamPayload};

mod catalog;
mod stream;

pub(in crate::acp) use catalog::refresh_headless_catalog;

const HEADLESS_TIMEOUT: Duration = Duration::from_secs(600);
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Text deltas arrive roughly one per token here. Each one used to cost an IPC serialize plus
/// a postMessage across the webview boundary, while the renderer coalesces everything into a
/// single store write per animation frame anyway — so that per-token cost bought nothing.
/// Same thresholds the native path already uses.
const HEADLESS_DELTA_BATCH_BYTES: usize = 4096;
const HEADLESS_DELTA_FLUSH_AFTER: Duration = Duration::from_millis(30);

/// Start-of-response latency is the one a reader actually notices, so the first delta of a
/// turn goes out on its own; only the mid-stream ones are worth coalescing.
fn should_flush_deltas(pending_bytes: usize, since_last: Duration, emitted_any: bool) -> bool {
    !emitted_any
        || pending_bytes >= HEADLESS_DELTA_BATCH_BYTES
        || since_last >= HEADLESS_DELTA_FLUSH_AFTER
}

fn is_claude_lifecycle_event(event_type: &str, nested_type: &str) -> bool {
    event_type == "stream_event"
        && matches!(
            nested_type,
            "message_start" | "message_stop" | "content_block_stop" | "ping"
        )
}

/// Accumulates text deltas and emits them as one `acp/delta` per batch. The payload shape is
/// unchanged — concatenated text is what the listener would have appended anyway.
struct DeltaBatch<'a> {
    app: &'a tauri::AppHandle,
    role: &'a str,
    runtime: &'static str,
    app_session_id: &'a str,
    turn_id: &'a str,
    buf: String,
    last_flush: Instant,
    emitted_any: bool,
}

impl<'a> DeltaBatch<'a> {
    fn new(
        app: &'a tauri::AppHandle,
        role: &'a str,
        runtime: &'static str,
        app_session_id: &'a str,
        turn_id: &'a str,
    ) -> Self {
        Self {
            app,
            role,
            runtime,
            app_session_id,
            turn_id,
            buf: String::new(),
            last_flush: Instant::now(),
            emitted_any: false,
        }
    }

    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buf.push_str(text);
        if should_flush_deltas(self.buf.len(), self.last_flush.elapsed(), self.emitted_any) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let _ = self.app.emit(
            "acp/delta",
            AcpDeltaPayload {
                schema_version: 1,
                role: self.role,
                runtime_kind: self.runtime,
                app_session_id: self.app_session_id,
                turn_id: self.turn_id,
                delta: &self.buf,
            },
        );
        self.buf.clear();
        self.last_flush = Instant::now();
        self.emitted_any = true;
    }

    /// Buffered text must reach the UI before the event it introduced, or a tool card renders
    /// above the sentence that announced it.
    fn emit(&mut self, sequence: &mut u32, event: AcpEvent) {
        self.flush();
        emit_event(
            self.app,
            self.role,
            self.runtime,
            self.app_session_id,
            self.turn_id,
            sequence,
            event,
        );
    }
}

/// Covers every exit the turn loop has — EOF, read error, a `result` frame, and a dropped
/// future on cancellation — without each one having to remember to drain the buffer.
impl Drop for DeltaBatch<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

pub(super) struct HeadlessTurn {
    pub(super) output: String,
    pub(super) response: Option<String>,
    pub(super) conversation_id: Option<String>,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
    pub(super) result_seen: bool,
}

static HEADLESS_LOCKS: OnceLock<dashmap::DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();
static HEADLESS_CHILDREN: OnceLock<dashmap::DashMap<String, u32>> = OnceLock::new();
static HEADLESS_CANCELLED: OnceLock<dashmap::DashSet<String>> = OnceLock::new();
static HEADLESS_ACTIVE: OnceLock<dashmap::DashMap<String, usize>> = OnceLock::new();

fn headless_locks() -> &'static dashmap::DashMap<String, Arc<Mutex<()>>> {
    HEADLESS_LOCKS.get_or_init(dashmap::DashMap::new)
}

fn headless_children() -> &'static dashmap::DashMap<String, u32> {
    HEADLESS_CHILDREN.get_or_init(dashmap::DashMap::new)
}

fn headless_cancelled() -> &'static dashmap::DashSet<String> {
    HEADLESS_CANCELLED.get_or_init(dashmap::DashSet::new)
}

fn headless_active() -> &'static dashmap::DashMap<String, usize> {
    HEADLESS_ACTIVE.get_or_init(dashmap::DashMap::new)
}

struct ActivityGuard {
    key: String,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut last = false;
        if let Some(mut count) = headless_active().get_mut(&self.key) {
            if *count <= 1 {
                drop(count);
                headless_active().remove(&self.key);
                last = true;
            } else {
                *count -= 1;
            }
        }
        if last {
            headless_cancelled().remove(&self.key);
        }
    }
}

pub(super) fn headless_key(runtime_key: &str, role_name: &str, app_session_id: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

pub(super) fn cancel_headless(runtime_key: &str, role_name: &str, app_session_id: &str) -> bool {
    let key = headless_key(runtime_key, role_name, app_session_id);
    let known = headless_active().contains_key(&key);
    if !known {
        return false;
    }
    headless_cancelled().insert(key.clone());
    super::perm_bridge::cancel_permissions_for_key(&key);
    if let Some((_, pid)) = headless_children().remove(&key) {
        terminate_pid(pid);
        acp_log(
            "headless.cancel",
            json!({ "runtime": runtime_key, "role": role_name, "pid": pid }),
        );
    }
    true
}

pub(super) async fn cancel_headless_and_wait(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    if !cancel_headless(runtime_key, role_name, app_session_id) {
        return Ok(());
    }
    let key = headless_key(runtime_key, role_name, app_session_id);
    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        while headless_active().contains_key(&key) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match drained {
        Ok(()) => {
            headless_cancelled().remove(&key);
            Ok(())
        }
        Err(_) => Err(format!("timed out waiting for {runtime_key} turn to stop")),
    }
}

pub(super) async fn discard_headless_session(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    stream::discard_session(runtime_key, role_name, app_session_id).await
}

pub(crate) async fn shutdown_headless_sessions() {
    let active_keys = headless_children()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for key in active_keys {
        headless_cancelled().insert(key.clone());
        if let Some(pid) = headless_children().get(&key).map(|entry| *entry) {
            terminate_pid(pid);
        }
    }
    stream::shutdown_sessions().await;
}

pub(crate) fn reclaim_idle_headless_sessions() {
    stream::reclaim_idle_sessions();
}

pub(super) async fn execute_headless_runtime(
    request: &super::adapter_runtime::PromptRequest<'_>,
) -> AcpPromptResult {
    let AdapterTransport::HeadlessJson {
        protocol,
        stream_input,
        output_format,
        conversation,
    } = request.transport
    else {
        unreachable!("headless executor requires headless transport");
    };

    if stream_input && output_format {
        return stream::execute_stream_runtime(request, protocol, conversation).await;
    }

    let runtime_key = request.runtime_key;
    let role_name = request.role_name;
    let prompt = request.prompt;
    let context = request.context;
    let cwd = request.cwd;
    let app = request.app;
    let auto_approve = request.auto_approve;
    let role_mode = request.role_mode;
    let role_config_options = request.role_config_options;
    let binary = request.binary;
    let adapter_args = request.adapter_args;
    let env = request.env;
    let resume_session_id = request.resume_session_id;
    let mcp_servers = request.mcp_servers;
    let app_session_id = request.app_session_id;
    let turn_id = request.turn_id;

    let key = headless_key(runtime_key, role_name, app_session_id);
    headless_active()
        .entry(key.clone())
        .and_modify(|count| *count += 1)
        .or_insert(1);
    let _activity = ActivityGuard { key: key.clone() };
    let lock = headless_locks()
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    if headless_cancelled().remove(&key).is_some() {
        return cancelled_result(
            runtime_key,
            role_name,
            app_session_id,
            turn_id,
            app,
            resume_session_id.map(str::to_string),
        );
    }

    let mut command_args = adapter_args.to_vec();
    append_cli_config(
        &mut command_args,
        runtime_key,
        protocol,
        role_config_options,
        role_mode,
    );
    if auto_approve {
        if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
            if let Some(config_path) = super::perm_bridge::claude_mcp_config_path(&key, mcp_servers)
            {
                command_args.push("--mcp-config".to_string());
                command_args.push(config_path);
            }
        }
        command_args.push("--dangerously-skip-permissions".to_string());
    } else if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
        // Permission-gated native Claude: wire prompts into the Jockey UI via
        // the permission MCP bridge. Never fall back to a silent bypass.
        match super::perm_bridge::claude_permission_args(&key, mcp_servers) {
            Ok(permission_args) => command_args.extend(permission_args),
            Err(error) => {
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    turn_id,
                    app,
                    AcpErrorCode::ConnectionFailed,
                    error,
                    json!({
                        "mode": "headless-json",
                        "transport": protocol_transport_name(protocol),
                        "runtimeKey": runtime_key,
                    }),
                );
            }
        }
    }
    if conversation {
        if let Some(session_id) = resume_session_id.as_deref().filter(|id| !id.is_empty()) {
            command_args.push(match protocol {
                HeadlessProtocol::AgyStreamJson => "--conversation".to_string(),
                HeadlessProtocol::ClaudeStreamJson => "--resume".to_string(),
            });
            command_args.push(session_id.to_string());
        }
    }
    if stream_input {
        command_args.extend(["--input-format".to_string(), "stream-json".to_string()]);
    }
    if output_format {
        command_args.extend(["--output-format".to_string(), "stream-json".to_string()]);
    }

    let prompt = compose_prompt(prompt, context);
    if !stream_input {
        command_args.push("--print".to_string());
        if matches!(protocol, HeadlessProtocol::ClaudeStreamJson)
            && output_format
            && !command_args.iter().any(|a| a == "--verbose")
        {
            command_args.push("--verbose".to_string());
        }
        command_args.push(prompt.clone());
    }

    acp_log(
        "headless.spawn.start",
        json!({
            "runtime": runtime_key,
            "binary": binary,
            "cwd": cwd,
            "streamInput": stream_input,
            "outputFormat": output_format,
            "mcpServerCount": mcp_servers.len(),
        }),
    );

    let mut command = tokio::process::Command::new(binary);
    command
        .args(&command_args)
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

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return error_result(
                runtime_key,
                role_name,
                app_session_id,
                turn_id,
                app,
                AcpErrorCode::ProcessCrashed,
                format!(
                    "failed to start {}: {error}",
                    protocol_display_name(protocol)
                ),
                json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
            )
        }
    };

    let pid = child.id();
    if let Some(pid) = pid {
        headless_children().insert(key.clone(), pid);
        register_child_pid(pid);
    }

    let stderr_buf = Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_task = child.stderr.take().map(|stderr| {
        let stderr_buf = stderr_buf.clone();
        let binary = binary.to_string();
        tokio::task::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        push_stderr_tail(&stderr_buf, &line);
                        if !line.trim().is_empty() {
                            acp_log(
                                "headless.stderr",
                                json!({ "binary": binary, "line": clip(line.trim(), 360) }),
                            );
                        }
                    }
                }
            }
        })
    });

    let mut stdin = child.stdin.take();
    if stream_input {
        let input = stream_input_frame(protocol, &prompt).to_string();
        if let Some(writer) = stdin.as_mut() {
            if let Err(error) = writer.write_all(format!("{input}\n").as_bytes()).await {
                cleanup_child(&key, pid, &mut child, stderr_task).await;
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    turn_id,
                    app,
                    AcpErrorCode::ConnectionFailed,
                    format!(
                        "failed to send prompt to {}: {error}",
                        protocol_display_name(protocol)
                    ),
                    json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
                );
            }
            if let Err(error) = writer.flush().await {
                cleanup_child(&key, pid, &mut child, stderr_task).await;
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    turn_id,
                    app,
                    AcpErrorCode::ConnectionFailed,
                    format!(
                        "failed to flush prompt to {}: {error}",
                        protocol_display_name(protocol)
                    ),
                    json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
                );
            }
        }
    }
    drop(stdin);

    let mut sequence = 0u32;
    if matches!(protocol, HeadlessProtocol::AgyStreamJson) && !mcp_servers.is_empty() {
        emit_event(
            app,
            role_name,
            runtime_key,
            app_session_id,
            turn_id,
            &mut sequence,
            AcpEvent::StatusUpdate {
                text: format!(
                    "{} MCP server(s) bound to this role are ignored: agy has no MCP mapping.",
                    mcp_servers.len()
                ),
            },
        );
        acp_log(
            "headless.agy.mcp_unmapped",
            json!({ "runtime": runtime_key, "count": mcp_servers.len() }),
        );
    }
    emit_event(
        app,
        role_name,
        runtime_key,
        app_session_id,
        turn_id,
        &mut sequence,
        AcpEvent::StatusUpdate {
            text: if let Some(ref sid) = resume_session_id.as_deref().filter(|s| !s.trim().is_empty()) {
                format!("Resuming {} session ({sid})...", protocol_display_name(protocol))
            } else {
                format!("Initializing new {} session...", protocol_display_name(protocol))
            },
        },
    );

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            cleanup_child(&key, pid, &mut child, stderr_task).await;
            return error_result(
                runtime_key,
                role_name,
                app_session_id,
                turn_id,
                app,
                AcpErrorCode::ConnectionFailed,
                format!("{} stdout unavailable", protocol_display_name(protocol)),
                json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
            );
        }
    };

    let read_result = tokio::time::timeout(
        HEADLESS_TIMEOUT,
        read_headless_output(
            stdout,
            protocol,
            output_format,
            app,
            role_name,
            runtime_key,
            app_session_id,
            turn_id,
            &mut sequence,
        ),
    )
    .await;

    let turn = match read_result {
        Ok(result) => result,
        Err(_) => {
            cleanup_child(&key, pid, &mut child, stderr_task).await;
            return error_result(
                runtime_key,
                role_name,
                app_session_id,
                turn_id,
                app,
                AcpErrorCode::PromptTimeout,
                format!(
                    "{} did not finish within {}s",
                    protocol_display_name(protocol),
                    HEADLESS_TIMEOUT.as_secs()
                ),
                json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
            );
        }
    };

    let status_result = if turn.result_seen {
        terminate_child(&mut child).await
    } else {
        match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => Some(status),
            Ok(Err(error)) => {
                cleanup_child(&key, pid, &mut child, stderr_task).await;
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    turn_id,
                    app,
                    AcpErrorCode::ProcessCrashed,
                    format!(
                        "failed to wait for {}: {error}",
                        protocol_display_name(protocol)
                    ),
                    json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
                );
            }
            Err(_) => terminate_child(&mut child).await,
        }
    };

    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    cleanup_pid(&key, pid);

    if headless_cancelled().remove(&key).is_some() {
        return cancelled_result(
            runtime_key,
            role_name,
            app_session_id,
            turn_id,
            app,
            turn.conversation_id
                .clone()
                .or_else(|| resume_session_id.map(str::to_string)),
        );
    }

    let stderr = stderr_tail(&stderr_buf);
    if let Some(error) = turn.error {
        let code = classify_error(&error);
        return error_result(
            runtime_key,
            role_name,
            app_session_id,
            turn_id,
            app,
            code,
            append_stderr(error, &stderr),
            json!({
                "mode": "headless-json",
                "runtimeKey": runtime_key,
                "conversationId": turn.conversation_id,
                "status": turn.status,
            }),
        );
    }

    if !turn.result_seen {
        if let Some(process_status) = status_result.filter(|status| !status.success()) {
            return error_result(
                runtime_key,
                role_name,
                app_session_id,
                turn_id,
                app,
                AcpErrorCode::ProcessCrashed,
                append_stderr(
                    format!(
                        "{} exited with {}",
                        protocol_display_name(protocol),
                        process_status
                    ),
                    &stderr,
                ),
                json!({
                    "mode": "headless-json",
                    "runtimeKey": runtime_key,
                    "conversationId": turn.conversation_id,
                    "status": turn.status,
                }),
            );
        }
    }

    let output = turn.response.unwrap_or(turn.output);
    acp_log(
        "headless.execute.ok",
        json!({
            "runtime": runtime_key,
            "role": role_name,
            "outputSize": output.len(),
            "conversationId": turn.conversation_id,
            "status": turn.status,
        }),
    );
    AcpPromptResult {
        ok: true,
        output,
        error_code: None,
        deltas: vec![],
        meta: json!({
            "mode": "headless-json",
            "transport": protocol_transport_name(protocol),
            "runtimeKey": runtime_key,
            "conversationId": turn.conversation_id,
            "status": turn.status,
        }),
        session_handle: turn.conversation_id,
    }
}

async fn read_headless_output(
    stdout: impl AsyncRead + Unpin,
    protocol: HeadlessProtocol,
    structured: bool,
    app: &tauri::AppHandle,
    role_name: &str,
    runtime_key: &'static str,
    app_session_id: &str,
    turn_id: &str,
    sequence: &mut u32,
) -> HeadlessTurn {
    let mut reader = BufReader::new(stdout);
    read_headless_turn(
        &mut reader,
        protocol,
        structured,
        app,
        role_name,
        runtime_key,
        app_session_id,
        turn_id,
        sequence,
    )
    .await
}

pub(super) async fn read_headless_turn<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    protocol: HeadlessProtocol,
    structured: bool,
    app: &tauri::AppHandle,
    role_name: &str,
    runtime_key: &'static str,
    app_session_id: &str,
    turn_id: &str,
    sequence: &mut u32,
) -> HeadlessTurn {
    let mut deltas = DeltaBatch::new(app, role_name, runtime_key, app_session_id, turn_id);
    let mut line = String::new();
    let mut raw_output = String::new();
    let mut response = None;
    let mut conversation_id = None;
    let mut status = None;
    let mut result_error = None;
    let mut result_seen = false;
    let mut claude_tool_inputs: HashMap<String, (String, String, String)> = HashMap::new();

    loop {
        line.clear();
        let read = match reader.read_line(&mut line).await {
            Ok(read) => read,
            Err(error) => {
                result_error = Some(format!("failed to read headless output: {error}"));
                break;
            }
        };
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !structured {
            raw_output.push_str(trimmed);
            raw_output.push('\n');
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            acp_log(
                "headless.stdout.unparsed",
                json!({ "runtime": runtime_key, "line": clip(trimmed, 360) }),
            );
            continue;
        };
        let event_type = event
            .get("type")
            .or_else(|| event.get("event"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let event_payload = event.get("event").filter(|value| value.is_object());
        let event_subtype = first_string(&event, &["subtype", "sub_type"]);
        let payload = match event_type.as_str() {
            "init" | "initialized" => event.get("init").or_else(|| event.get("initialized")),
            "step_update" | "step-update" => {
                event.get("step_update").or_else(|| event.get("stepUpdate"))
            }
            "result" | "completed" | "complete" => {
                event.get("result").or_else(|| event.get("completed"))
            }
            _ => None,
        }
        .unwrap_or(&event);

        if let Some(id) = first_string(
            &event,
            &[
                "conversation_id",
                "conversationId",
                "session_id",
                "sessionId",
            ],
        )
        .or_else(|| {
            first_string(
                payload,
                &[
                    "conversation_id",
                    "conversationId",
                    "session_id",
                    "sessionId",
                ],
            )
        }) {
            conversation_id = Some(id);
        }

        if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
            // Sub-agent turns (the Task tool) carry the spawning tool call's id, which lets
            // the UI nest their tool calls instead of flattening everything to one level.
            let parent_tool_use_id =
                first_string(&event, &["parent_tool_use_id", "parentToolUseId"]);
            let nested_type = event_payload
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            match event_type.as_str() {
                "rate_limit_event" => {
                    if let Some(text) = event
                        .get("rate_limit")
                        .or_else(|| event.get("message"))
                        .and_then(value_text)
                    {
                        deltas.emit(
                            sequence,
                            AcpEvent::Notice {
                                level: "warning".to_string(),
                                code: Some("rate_limit".to_string()),
                                text,
                            },
                        );
                    }
                }
                "system" if event_subtype.as_deref() == Some("compact_boundary") => {
                    let at = event.get("compact_metadata");
                    deltas.emit(
                        sequence,
                        AcpEvent::ContextCompacted {
                            reason: at
                                .and_then(|m| m.get("trigger"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            before_tokens: at
                                .and_then(|m| m.get("pre_tokens"))
                                .and_then(Value::as_u64),
                            after_tokens: None,
                        },
                    );
                }
                "system" => {
                    if event_subtype.as_deref() == Some("init") {
                        deltas.emit(
                            sequence,
                            AcpEvent::StatusUpdate {
                                text: "Claude session ready".to_string(),
                            },
                        );
                    }
                }
                "stream_event" if nested_type == "message_delta" => {
                    if let Some(usage) = event_payload
                        .and_then(|value| value.get("usage"))
                        .filter(|u| has_usage_counters(u))
                    {
                        deltas.emit(sequence, usage_event(usage, None, None));
                    }
                }
                // Claude lifecycle frames carry provider metadata only. They are not
                // conversation content and must not become raw debug blocks in the UI.
                "stream_event" if is_claude_lifecycle_event(&event_type, &nested_type) => {}
                "stream_event" if nested_type == "content_block_start" => {
                    let index = event
                        .get("index")
                        .or_else(|| event_payload.and_then(|value| value.get("index")))
                        .map(Value::to_string);
                    let block = event_payload.and_then(|value| value.get("content_block"));
                    if let (Some(index), Some(block)) = (index, block) {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            if let (Some(id), Some(name)) =
                                (first_string(block, &["id"]), first_string(block, &["name"]))
                            {
                                claude_tool_inputs
                                    .insert(index, (id.clone(), name.clone(), String::new()));
                                deltas.emit(
                                    sequence,
                                    AcpEvent::ToolCallUpdate {
                                        tool_call_id: id,
                                        tool_name: Some(name.clone()),
                                        tool_kind: Some("tool".to_string()),
                                        status: Some("running".to_string()),
                                        title: Some(name),
                                        content: None,
                                        locations: None,
                                        raw_input: block.get("input").cloned(),
                                        raw_output: None,
                                        terminal_meta: None,
                                        parent_id: parent_tool_use_id.clone(),
                                        diff: None,
                                    },
                                );
                            }
                        }
                    }
                }
                "stream_event" if nested_type == "content_block_delta" => {
                    let delta_value = event_payload.and_then(|value| value.get("delta"));
                    let delta_type = delta_value
                        .and_then(|value| value.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match delta_type {
                        "input_json_delta" => {
                            let index = event
                                .get("index")
                                .or_else(|| event_payload.and_then(|value| value.get("index")))
                                .map(Value::to_string);
                            let partial = delta_value
                                .and_then(|value| value.get("partial_json"))
                                .and_then(Value::as_str);
                            if let (Some(index), Some(partial)) = (index, partial) {
                                if let Some((tool_id, tool_name, buffer)) =
                                    claude_tool_inputs.get_mut(&index)
                                {
                                    buffer.push_str(partial);
                                    if let Ok(input) = serde_json::from_str::<Value>(buffer) {
                                        deltas.emit(
                                            sequence,
                                            AcpEvent::ToolCallUpdate {
                                                tool_call_id: tool_id.clone(),
                                                tool_name: Some(tool_name.clone()),
                                                tool_kind: Some("tool".to_string()),
                                                status: Some("running".to_string()),
                                                title: Some(tool_name.clone()),
                                                content: None,
                                                locations: None,
                                                raw_input: Some(input),
                                                raw_output: None,
                                                terminal_meta: None,
                                                parent_id: parent_tool_use_id.clone(),
                                                diff: None,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        "thinking_delta" => {
                            if let Some(text) = delta_value
                                .and_then(|value| value.get("thinking"))
                                .and_then(Value::as_str)
                                .filter(|text| !text.is_empty())
                            {
                                deltas.emit(
                                    sequence,
                                    AcpEvent::ThoughtDelta {
                                        text: text.to_string(),
                                    },
                                );
                            }
                        }
                        _ => {
                            let delta = delta_value
                                .and_then(|value| value.get("text"))
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if !delta.is_empty() {
                                raw_output.push_str(delta);
                                deltas.push(delta);
                            }
                        }
                    }
                }
                // Tool results arrive as `user` frames. Without this arm every tool card
                // emitted by the `assistant` arm stays at `running` forever and never shows
                // its output.
                "user" => {
                    if let Some(content) = event
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                    {
                        for block in content {
                            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                                continue;
                            }
                            let Some(tool_call_id) =
                                first_string(block, &["tool_use_id", "toolUseId"])
                            else {
                                continue;
                            };
                            let is_error = block
                                .get("is_error")
                                .or_else(|| block.get("isError"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            deltas.emit(
                                sequence,
                                AcpEvent::ToolCallUpdate {
                                    tool_call_id,
                                    tool_name: None,
                                    tool_kind: None,
                                    status: Some(
                                        if is_error { "failure" } else { "completed" }.to_string(),
                                    ),
                                    title: None,
                                    content: None,
                                    locations: None,
                                    raw_input: None,
                                    raw_output: block.get("content").cloned(),
                                    terminal_meta: None,
                                    parent_id: None,
                                    diff: None,
                                },
                            );
                        }
                    }
                }
                "assistant" => {
                    if let Some(content) = event
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                    {
                        for block in content {
                            match block.get("type").and_then(Value::as_str) {
                                Some("thinking") => {
                                    if let Some(text) = block
                                        .get("thinking")
                                        .and_then(Value::as_str)
                                        .filter(|text| !text.is_empty())
                                    {
                                        deltas.emit(
                                            sequence,
                                            AcpEvent::ThoughtDelta {
                                                text: text.to_string(),
                                            },
                                        );
                                    }
                                }
                                Some("tool_use") => {
                                    // TodoWrite is Claude's plan surface; route it to the plan
                                    // panel instead of rendering it as one more tool card.
                                    if first_string(block, &["name"]).as_deref()
                                        == Some("TodoWrite")
                                    {
                                        if let Some(entries) = block
                                            .get("input")
                                            .and_then(|input| input.get("todos"))
                                            .and_then(Value::as_array)
                                        {
                                            deltas.emit(
                                                sequence,
                                                AcpEvent::Plan {
                                                    entries: entries.clone(),
                                                },
                                            );
                                            continue;
                                        }
                                    }
                                    deltas.emit(
                                        sequence,
                                        AcpEvent::ToolCallUpdate {
                                            tool_call_id: first_string(block, &["id"])
                                                .unwrap_or_else(|| "claude-tool".to_string()),
                                            tool_name: first_string(block, &["name"]),
                                            tool_kind: Some("tool".to_string()),
                                            status: Some("running".to_string()),
                                            title: first_string(block, &["name"]),
                                            content: None,
                                            locations: None,
                                            raw_input: block.get("input").cloned(),
                                            raw_output: None,
                                            terminal_meta: None,
                                            parent_id: parent_tool_use_id.clone(),
                                            diff: None,
                                        },
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "result" => {
                    result_seen = true;
                    if let Some(usage) = event.get("usage").filter(|u| has_usage_counters(u)) {
                        let cost = event
                            .get("total_cost_usd")
                            .or_else(|| event.get("totalCostUsd"))
                            .and_then(Value::as_f64);
                        deltas.emit(sequence, usage_event(usage, cost, None));
                    }
                    response = event.get("result").and_then(value_text);
                    status = event_subtype.or_else(|| first_string(&event, &["status"]));
                    if event
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        result_error = event
                            .get("error")
                            .or_else(|| event.get("result"))
                            .and_then(value_text)
                            .or_else(|| Some("Claude returned an error result".to_string()));
                    }
                    break;
                }
                _ if !event_type.is_empty() => {
                    deltas.emit(
                        sequence,
                        AcpEvent::Unknown {
                            type_name: event_type.clone(),
                            raw: event.clone(),
                        },
                    );
                }
                _ => {}
            }
            continue;
        }

        match event_type.as_str() {
            "init" | "initialized" => {
                deltas.emit(
                    sequence,
                    AcpEvent::StatusUpdate {
                        text: "Antigravity session initialized".to_string(),
                    },
                );
            }
            "step_update" | "step-update" => {
                if let Some(delta) = first_string(payload, &["text_delta", "textDelta"]) {
                    raw_output.push_str(&delta);
                    deltas.push(&delta);
                }
                if let Some(usage) = payload.get("usage").filter(|u| has_usage_counters(u)) {
                    deltas.emit(sequence, usage_event(usage, None, None));
                }
                let step_type =
                    first_string(payload, &["step_type", "stepType"]).unwrap_or_default();
                let tool_info = payload.get("tool_info").or_else(|| payload.get("toolInfo"));
                let has_tool_detail = first_string(payload, &["tool_name", "toolName"]).is_some()
                    || tool_info.is_some();
                // Antigravity's print mode reports most steps as `step_type: "unknown"` with no
                // name, parameters or output. Rendering those as tool cards produces a wall of
                // blank "unknown" entries, so only surface steps that carry real detail.
                let is_renderable = step_type != "agent_response"
                    && step_type != "user_input"
                    && (has_tool_detail || step_type != "unknown");
                if is_renderable {
                    let tool_id = payload
                        .get("step_index")
                        .or_else(|| payload.get("stepIndex"))
                        .map(Value::to_string)
                        .unwrap_or_else(|| "0".to_string());
                    deltas.emit(
                        sequence,
                        AcpEvent::ToolCallUpdate {
                            tool_call_id: format!("agy-step-{tool_id}"),
                            tool_name: first_string(payload, &["tool_name", "toolName"]).or_else(
                                || {
                                    tool_info
                                        .and_then(|info| first_string(info, &["name", "toolName"]))
                                },
                            ),
                            tool_kind: Some("tool".to_string()),
                            status: first_string(payload, &["state", "status"]).map(|status| {
                                match status.to_ascii_lowercase().as_str() {
                                    "active" | "running" | "in_progress" | "inprogress" => {
                                        "running".to_string()
                                    }
                                    "done" | "completed" | "success" => "completed".to_string(),
                                    "failed" | "failure" | "error" => "failure".to_string(),
                                    other => other.to_string(),
                                }
                            }),
                            title: first_string(
                                payload,
                                &["tool_name", "toolName", "step_type", "stepType"],
                            ),
                            content: None,
                            locations: None,
                            raw_input: payload
                                .get("parameters")
                                .or_else(|| payload.get("input"))
                                .or_else(|| tool_info.and_then(|info| info.get("parameters")))
                                .cloned(),
                            raw_output: payload
                                .get("output")
                                .or_else(|| payload.get("result"))
                                .or_else(|| tool_info.and_then(|info| info.get("output")))
                                .cloned(),
                            terminal_meta: None,
                            parent_id: None,
                            diff: None,
                        },
                    );
                }
            }
            "text_delta" | "text-delta" => {
                if let Some(delta) = first_string(payload, &["text", "text_delta", "textDelta"]) {
                    raw_output.push_str(&delta);
                    deltas.push(&delta);
                }
            }
            "result" | "completed" | "complete" => {
                result_seen = true;
                response = payload
                    .get("response")
                    .or_else(|| payload.get("output"))
                    .and_then(value_text);
                status = first_string(payload, &["status", "state"]);
                result_error = payload
                    .get("error")
                    .or_else(|| payload.get("message"))
                    .and_then(value_text)
                    .or_else(|| {
                        status
                            .as_deref()
                            .filter(|value| {
                                matches!(
                                    value.to_ascii_lowercase().as_str(),
                                    "error" | "failed" | "failure" | "cancelled" | "canceled"
                                )
                            })
                            .map(|value| format!("headless runtime returned status {value}"))
                    });
                break;
            }
            _ if !event_type.is_empty() => {
                deltas.emit(
                    sequence,
                    AcpEvent::Unknown {
                        type_name: event_type.clone(),
                        raw: event.clone(),
                    },
                );
            }
            _ => {}
        }
    }

    HeadlessTurn {
        output: raw_output.trim().to_string(),
        response,
        conversation_id,
        status,
        error: result_error,
        result_seen,
    }
}

/// The pre-declaration behaviour, kept only for a turn that runs before discovery has
/// populated the catalog in this process. It guesses key spellings, which is precisely what
/// the declared path exists to stop doing.
fn append_guessed_config(
    args: &mut Vec<String>,
    protocol: HeadlessProtocol,
    options: &[(String, String)],
) {
    if let Some(model) = find_option(options, &["model", "model_id"]) {
        let one_million = find_option(options, &["one_million", "oneMillion", "1m"])
            .is_some_and(|value| toggle_is_on(&value));
        let model = if one_million && !model.ends_with("[1m]") {
            format!("{model}[1m]")
        } else {
            model
        };
        args.extend(["--model".to_string(), model]);
    }
    if let Some(effort) = find_option(options, &["effort", "reasoning_effort", "reasoningEffort"])
        .and_then(|effort| clamp_effort(protocol, &effort))
    {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    if matches!(protocol, HeadlessProtocol::ClaudeStreamJson)
        && find_option(options, &["fast", "fast_mode", "fastMode"])
            .is_some_and(|value| toggle_is_on(&value))
    {
        args.extend([
            "--settings".to_string(),
            json!({ "fastMode": true }).to_string(),
        ]);
    }
}

/// True for the strings a stored toggle uses for "on".
fn toggle_is_on(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "on" | "yes")
}

/// Applies the runtime's *declared* parameters. Before this, the launcher guessed every key
/// (`["one_million", "oneMillion", "1m"]`) and hard-coded each delivery mechanism, including a
/// `matches!(protocol, ClaudeStreamJson)` branch for fast mode — so a capability could be
/// discovered and still never reach the CLI. The catalog now says both the key and the
/// mechanism; this only obeys it. Returns false when nothing was declared yet, so the caller
/// can fall back rather than drop the user's settings on a turn that runs before discovery.
fn append_declared_config(
    args: &mut Vec<String>,
    runtime_key: &str,
    protocol: HeadlessProtocol,
    options: &[(String, String)],
) -> bool {
    let specs = crate::acp::runtime_state::list_discovered_config_options(runtime_key);
    if specs.is_empty() {
        return false;
    }

    let stored = |id: &str| -> Option<String> {
        options
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.trim().is_empty())
    };

    let mut model = specs
        .iter()
        .find(|spec| spec.get("id").and_then(Value::as_str) == Some("model"))
        .and_then(|_| stored("model"));
    let mut flags: Vec<(String, String)> = Vec::new();
    let mut settings = serde_json::Map::new();

    for spec in &specs {
        let Some(id) = spec.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id == "model" {
            continue;
        }
        let Some(value) = stored(id) else { continue };
        let is_toggle = spec.get("kind").and_then(Value::as_str) == Some("toggle");
        if is_toggle && !toggle_is_on(&value) {
            continue;
        }
        let Some(wire) = spec.get("wire") else {
            continue;
        };
        match wire.get("kind").and_then(Value::as_str) {
            Some("cli_flag") => {
                if let Some(flag) = wire.get("flag").and_then(Value::as_str) {
                    // Antigravity tops out below Claude, so a level carried over from another
                    // engine is clamped rather than passed through and rejected.
                    let value = if flag == "--effort" {
                        match clamp_effort(protocol, &value) {
                            Some(clamped) => clamped.to_string(),
                            None => continue,
                        }
                    } else {
                        value
                    };
                    flags.push((flag.to_string(), value));
                }
            }
            Some("model_suffix") => {
                if let (Some(suffix), Some(current)) =
                    (wire.get("suffix").and_then(Value::as_str), model.as_ref())
                {
                    if !current.ends_with(suffix) {
                        model = Some(format!("{current}{suffix}"));
                    }
                }
            }
            Some("cli_settings") => {
                if let Some(Value::Object(json)) = wire.get("json") {
                    for (key, value) in json {
                        settings.insert(key.clone(), value.clone());
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(model) = model {
        args.extend(["--model".to_string(), model]);
    }
    for (flag, value) in flags {
        args.extend([flag, value]);
    }
    if !settings.is_empty() {
        args.extend([
            "--settings".to_string(),
            Value::Object(settings).to_string(),
        ]);
    }
    true
}

pub(super) fn append_cli_config(
    args: &mut Vec<String>,
    runtime_key: &str,
    protocol: HeadlessProtocol,
    options: &[(String, String)],
    mode: Option<&str>,
) {
    if !append_declared_config(args, runtime_key, protocol, options) {
        append_guessed_config(args, protocol, options);
    }
    if let Some(mode) = mode.and_then(normalize_mode) {
        let (flag, value) = match protocol {
            HeadlessProtocol::AgyStreamJson => ("--mode", mode),
            HeadlessProtocol::ClaudeStreamJson => {
                ("--permission-mode", claude_permission_mode(mode))
            }
        };
        args.extend([flag.to_string(), value.to_string()]);
    }
}

/// Roles carry one effort value across runtimes, but the accepted levels differ:
/// Claude takes low..max, Antigravity stops at high. Clamp rather than pass a value the
/// CLI will reject, which would fail the whole turn.
fn clamp_effort(protocol: HeadlessProtocol, effort: &str) -> Option<&'static str> {
    let level = match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" | "x-high" => "xhigh",
        "max" | "ultracode" => "max",
        _ => return None,
    };
    match protocol {
        HeadlessProtocol::ClaudeStreamJson => Some(level),
        HeadlessProtocol::AgyStreamJson => Some(match level {
            "xhigh" | "max" => "high",
            other => other,
        }),
    }
}

/// Map a provider usage object onto the shared `Usage` event. Providers spell the same
/// counters differently; anything absent stays `None` so the UI can distinguish
/// "not reported" from zero.
fn usage_event(usage: &Value, cost_usd: Option<f64>, context_window: Option<u64>) -> AcpEvent {
    let pick = |names: &[&str]| -> Option<u64> {
        names
            .iter()
            .find_map(|name| usage.get(*name).and_then(Value::as_u64))
    };
    AcpEvent::Usage {
        input_tokens: pick(&["input_tokens", "inputTokens"]),
        output_tokens: pick(&["output_tokens", "outputTokens"]),
        cache_read_tokens: pick(&[
            "cache_read_input_tokens",
            "cache_read_tokens",
            "cachedInputTokens",
        ]),
        cache_write_tokens: pick(&[
            "cache_creation_input_tokens",
            "cache_write_tokens",
            "cacheWriteInputTokens",
        ]),
        reasoning_tokens: pick(&["thinking_tokens", "reasoningOutputTokens"]),
        total_tokens: pick(&["total_tokens", "totalTokens"]),
        context_window,
        cost_usd,
    }
}

fn has_usage_counters(usage: &Value) -> bool {
    usage
        .as_object()
        .is_some_and(|map| map.values().any(|v| v.as_u64().is_some_and(|n| n > 0)))
}

fn claude_permission_mode(mode: &str) -> &str {
    match mode {
        "accept-edits" => "acceptEdits",
        other => other,
    }
}

fn stream_input_frame(protocol: HeadlessProtocol, prompt: &str) -> Value {
    match protocol {
        HeadlessProtocol::AgyStreamJson => json!({
            "event": "user",
            "message": { "content": prompt },
        }),
        HeadlessProtocol::ClaudeStreamJson => json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": prompt }],
            },
        }),
    }
}

pub(super) fn protocol_display_name(protocol: HeadlessProtocol) -> &'static str {
    match protocol {
        HeadlessProtocol::AgyStreamJson => "Antigravity CLI",
        HeadlessProtocol::ClaudeStreamJson => "Claude Code",
    }
}

pub(super) fn protocol_transport_name(protocol: HeadlessProtocol) -> &'static str {
    match protocol {
        HeadlessProtocol::AgyStreamJson => "agy-stream-json",
        HeadlessProtocol::ClaudeStreamJson => "claude-stream-json",
    }
}

fn find_option(options: &[(String, String)], names: &[&str]) -> Option<String> {
    options
        .iter()
        .find(|(key, value)| {
            !value.trim().is_empty() && names.iter().any(|name| key.eq_ignore_ascii_case(name))
        })
        .map(|(_, value)| value.clone())
}

fn normalize_mode(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "plan" => Some("plan"),
        "accept-edits" | "accept_edits" | "accept edits" | "edit" | "auto" => Some("accept-edits"),
        _ => None,
    }
}

fn compose_prompt(prompt: &str, context: &[(String, String)]) -> String {
    if context.is_empty() {
        return prompt.to_string();
    }
    let context = context
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{prompt}\n\n[context]\n{context}")
}

fn emit_event(
    app: &tauri::AppHandle,
    role_name: &str,
    runtime_key: &'static str,
    app_session_id: &str,
    turn_id: &str,
    sequence: &mut u32,
    event: AcpEvent,
) {
    *sequence = sequence.saturating_add(1);
    let _ = app.emit(
        "acp/stream",
        AcpStreamPayload {
            schema_version: 1,
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
            turn_id,
            event: &event,
            seq: *sequence,
        },
    );
}

fn first_string(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(value_text))
}

fn value_text(value: &Value) -> Option<String> {
    value.as_str().map(ToString::to_string).or_else(|| {
        value
            .get("text")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn classify_error(error: &str) -> AcpErrorCode {
    let lower = error.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("login")
        || lower.contains("credential")
        || lower.contains("api key")
    {
        AcpErrorCode::AuthRequired
    } else if lower.contains("cancel") {
        AcpErrorCode::RequestCancelled
    } else {
        AcpErrorCode::AgentError
    }
}

fn append_stderr(mut message: String, stderr: &str) -> String {
    if !stderr.trim().is_empty() {
        message.push_str(": ");
        message.push_str(stderr.trim());
    }
    message
}

fn error_result(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
    turn_id: &str,
    app: &tauri::AppHandle,
    code: AcpErrorCode,
    raw: impl Into<String>,
    meta: Value,
) -> AcpPromptResult {
    let raw = raw.into();
    let message = friendly_error_message(runtime_key, &raw);
    let event = AcpEvent::SessionError {
        code: code.as_str().to_string(),
        message: raw.clone(),
        retryable: !matches!(
            code,
            AcpErrorCode::AuthRequired | AcpErrorCode::InvalidParams
        ),
    };
    let _ = app.emit(
        "acp/stream",
        AcpStreamPayload {
            schema_version: 1,
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
            turn_id,
            event: &event,
            seq: 0,
        },
    );
    acp_log(
        "headless.execute.error",
        json!({ "runtime": runtime_key, "role": role_name, "code": code.as_str(), "error": raw }),
    );
    AcpPromptResult {
        ok: false,
        output: message,
        error_code: Some(code.as_str().to_string()),
        deltas: vec![],
        meta,
        session_handle: None,
    }
}

pub(super) fn cancelled_result(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
    turn_id: &str,
    app: &tauri::AppHandle,
    conversation_id: Option<String>,
) -> AcpPromptResult {
    let mut result = error_result(
        runtime_key,
        role_name,
        app_session_id,
        turn_id,
        app,
        AcpErrorCode::RequestCancelled,
        "headless prompt cancelled",
        json!({
            "mode": "headless-json",
            "runtimeKey": runtime_key,
            "sessionId": conversation_id.as_deref(),
            "conversationId": conversation_id.as_deref(),
        }),
    );
    result.session_handle = conversation_id;
    result
}

async fn cleanup_child(
    key: &str,
    pid: Option<u32>,
    child: &mut Child,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
) {
    terminate_child(child).await;
    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    cleanup_pid(key, pid);
}

async fn terminate_child(child: &mut Child) -> Option<std::process::ExitStatus> {
    if let Some(pid) = child.id() {
        terminate_pid(pid);
    }
    let _ = child.kill().await;
    child.wait().await.ok()
}

fn cleanup_pid(key: &str, pid: Option<u32>) {
    if let Some(pid) = pid {
        if headless_children()
            .get(key)
            .map(|current| *current == pid)
            .unwrap_or(false)
        {
            headless_children().remove(key);
        }
        unregister_child_pid(pid);
    }
}

pub(super) use crate::acp::process::terminate_pid;

#[cfg(test)]
mod append_cli_config_tests {
    use std::time::Duration;

    #[test]
    fn the_first_delta_of_a_turn_is_never_held_back() {
        // Start-of-response latency is the one a reader notices; batching it would make the
        // model look slower to start than it is.
        assert!(super::should_flush_deltas(
            1,
            Duration::from_millis(0),
            false
        ));
    }

    #[test]
    fn claude_message_lifecycle_frames_are_not_conversation_blocks() {
        assert!(super::is_claude_lifecycle_event(
            "stream_event",
            "message_start"
        ));
        assert!(super::is_claude_lifecycle_event(
            "stream_event",
            "message_stop"
        ));
        assert!(super::is_claude_lifecycle_event(
            "stream_event",
            "content_block_stop"
        ));
        assert!(super::is_claude_lifecycle_event("stream_event", "ping"));
        assert!(!super::is_claude_lifecycle_event(
            "stream_event",
            "content_block_delta"
        ));
        assert!(!super::is_claude_lifecycle_event(
            "assistant",
            "message_start"
        ));
    }

    #[test]
    fn mid_stream_deltas_coalesce_until_a_threshold_is_hit() {
        let emitted = true;
        assert!(!super::should_flush_deltas(
            8,
            Duration::from_millis(1),
            emitted
        ));
        assert!(super::should_flush_deltas(
            super::HEADLESS_DELTA_BATCH_BYTES,
            Duration::from_millis(1),
            emitted
        ));
        assert!(super::should_flush_deltas(
            8,
            super::HEADLESS_DELTA_FLUSH_AFTER,
            emitted
        ));
    }

    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn usage_fields(event: &AcpEvent) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        match event {
            AcpEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                reasoning_tokens,
                ..
            } => (
                *input_tokens,
                *output_tokens,
                *cache_read_tokens,
                *reasoning_tokens,
            ),
            _ => panic!("expected a Usage event"),
        }
    }

    #[test]
    fn maps_claude_usage_spelling() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 20,
            "cache_read_input_tokens": 7,
            "cache_creation_input_tokens": 3
        });
        assert_eq!(
            usage_fields(&usage_event(&usage, Some(0.25), None)),
            (Some(100), Some(20), Some(7), None)
        );
    }

    #[test]
    fn maps_antigravity_usage_spelling() {
        let usage = json!({
            "input_tokens": 14068,
            "output_tokens": 364,
            "thinking_tokens": 304,
            "cache_read_tokens": 11,
            "total_tokens": 14432
        });
        assert_eq!(
            usage_fields(&usage_event(&usage, None, None)),
            (Some(14068), Some(364), Some(11), Some(304))
        );
    }

    #[test]
    fn unreported_counters_stay_none_rather_than_zero() {
        let usage = json!({ "input_tokens": 5 });
        let (_, output, cache, reasoning) = usage_fields(&usage_event(&usage, None, None));
        assert_eq!((output, cache, reasoning), (None, None, None));
    }

    #[test]
    fn all_zero_usage_is_not_worth_emitting() {
        assert!(!has_usage_counters(
            &json!({ "input_tokens": 0, "output_tokens": 0 })
        ));
        assert!(has_usage_counters(&json!({ "input_tokens": 1 })));
    }

    /// The point of the refactor: the launcher reads what the runtime declared rather than
    /// guessing key spellings and hard-coding each delivery mechanism. All three of Claude's
    /// mechanisms are exercised here — a flag, a model-id suffix and a settings blob — none of
    /// which the launcher knows about by name any more.
    #[test]
    fn a_declared_catalog_drives_every_delivery_mechanism() {
        let runtime = "declared-headless-probe";
        crate::acp::runtime_state::remember_runtime_config_options(
            runtime,
            vec![
                json!({ "id": "model", "kind": "select",
                        "wire": { "kind": "cli_flag", "flag": "--model" } }),
                json!({ "id": "effort", "kind": "select",
                        "wire": { "kind": "cli_flag", "flag": "--effort" } }),
                json!({ "id": "one_million", "kind": "toggle",
                        "wire": { "kind": "model_suffix", "suffix": "[1m]" } }),
                json!({ "id": "fast", "kind": "toggle",
                        "wire": { "kind": "cli_settings", "json": { "fastMode": true } } }),
            ],
        );

        let mut args = vec![];
        append_cli_config(
            &mut args,
            runtime,
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[
                ("model", "claude-opus-5"),
                ("effort", "xhigh"),
                ("one_million", "true"),
                ("fast", "true"),
            ]),
            None,
        );
        assert_eq!(
            args,
            vec![
                "--model",
                "claude-opus-5[1m]",
                "--effort",
                "xhigh",
                "--settings",
                r#"{"fastMode":true}"#
            ]
        );

        // An off toggle delivers nothing at all, rather than an empty value.
        let mut off = vec![];
        append_cli_config(
            &mut off,
            runtime,
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[("model", "claude-opus-5"), ("fast", "")]),
            None,
        );
        assert_eq!(off, vec!["--model", "claude-opus-5"]);

        crate::acp::runtime_state::clear_runtime(runtime);
    }

    #[test]
    fn one_million_toggle_suffixes_the_model_id() {
        let mut args = vec![];
        append_cli_config(
            &mut args,
            "test-runtime",
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[("model", "claude-opus-5"), ("one_million", "true")]),
            None,
        );
        assert_eq!(args, vec!["--model", "claude-opus-5[1m]"]);
    }

    #[test]
    fn one_million_toggle_does_not_double_suffix() {
        let mut args = vec![];
        append_cli_config(
            &mut args,
            "test-runtime",
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[("model", "claude-opus-5[1m]"), ("one_million", "true")]),
            None,
        );
        assert_eq!(args, vec!["--model", "claude-opus-5[1m]"]);
    }

    #[test]
    fn fast_mode_goes_through_settings_json_not_a_flag() {
        let mut args = vec![];
        append_cli_config(
            &mut args,
            "test-runtime",
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[("fast", "true")]),
            None,
        );
        assert_eq!(args, vec!["--settings", r#"{"fastMode":true}"#]);
    }

    #[test]
    fn antigravity_ignores_fast_and_clamps_effort() {
        let mut args = vec![];
        append_cli_config(
            &mut args,
            "test-runtime",
            HeadlessProtocol::AgyStreamJson,
            &opts(&[("fast", "true"), ("effort", "xhigh")]),
            None,
        );
        assert_eq!(args, vec!["--effort", "high"]);
    }

    #[test]
    fn claude_keeps_its_full_effort_range() {
        let mut args = vec![];
        append_cli_config(
            &mut args,
            "test-runtime",
            HeadlessProtocol::ClaudeStreamJson,
            &opts(&[("effort", "xhigh")]),
            None,
        );
        assert_eq!(args, vec!["--effort", "xhigh"]);
    }
}
