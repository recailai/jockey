use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::Emitter;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

use super::super::adapter::{
    acp_log, clip, friendly_error_message, AdapterTransport, HeadlessProtocol,
};
use super::super::error::{push_stderr_tail, stderr_tail, AcpErrorCode};
use super::super::protocol as acp;
use super::super::worker::{register_child_pid, unregister_child_pid, AcpEvent, AcpPromptResult};
use super::execute::{AcpDeltaPayload, AcpStreamPayload};

mod stream;

const HEADLESS_TIMEOUT: Duration = Duration::from_secs(600);
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

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

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_headless_runtime(
    transport: AdapterTransport,
    runtime_key: &'static str,
    role_name: &str,
    prompt: &str,
    context: &[(String, String)],
    cwd: &str,
    app: &tauri::AppHandle,
    auto_approve: bool,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    binary: &str,
    adapter_args: &[String],
    env: &[(String, String)],
    resume_session_id: Option<String>,
    mcp_servers: &[acp::McpServer],
    app_session_id: &str,
) -> AcpPromptResult {
    let AdapterTransport::HeadlessJson {
        protocol,
        stream_input,
        output_format,
        conversation,
    } = transport
    else {
        unreachable!("headless executor requires headless transport");
    };

    if stream_input && output_format {
        return stream::execute_stream_runtime(
            protocol,
            runtime_key,
            role_name,
            prompt,
            context,
            cwd,
            app,
            auto_approve,
            role_mode,
            role_config_options,
            binary,
            adapter_args,
            env,
            conversation,
            resume_session_id,
            mcp_servers,
            app_session_id,
        )
        .await;
    }

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
        return cancelled_result(runtime_key, role_name, app_session_id, app);
    }

    let mut command_args = adapter_args.to_vec();
    append_cli_config(
        &mut command_args,
        protocol,
        &role_config_options,
        role_mode.as_deref(),
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
        &mut sequence,
        AcpEvent::StatusUpdate {
            text: format!("Connected to {}", protocol_display_name(protocol)),
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
        return cancelled_result(runtime_key, role_name, app_session_id, app);
    }

    let stderr = stderr_tail(&stderr_buf);
    if let Some(error) = turn.error {
        let code = classify_error(&error);
        return error_result(
            runtime_key,
            role_name,
            app_session_id,
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
    sequence: &mut u32,
) -> HeadlessTurn {
    let mut line = String::new();
    let mut raw_output = String::new();
    let mut response = None;
    let mut conversation_id = None;
    let mut status = None;
    let mut result_error = None;
    let mut result_seen = false;

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
            let nested_type = event_payload
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            match event_type.as_str() {
                "system" => {
                    if event_subtype.as_deref() == Some("init") {
                        emit_event(
                            app,
                            role_name,
                            runtime_key,
                            app_session_id,
                            sequence,
                            AcpEvent::StatusUpdate {
                                text: "Claude session initialized".to_string(),
                            },
                        );
                    }
                }
                "stream_event" if nested_type == "content_block_delta" => {
                    let delta_value = event_payload.and_then(|value| value.get("delta"));
                    let delta_type = delta_value
                        .and_then(|value| value.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match delta_type {
                        "thinking_delta" => {
                            if let Some(text) = delta_value
                                .and_then(|value| value.get("thinking"))
                                .and_then(Value::as_str)
                                .filter(|text| !text.is_empty())
                            {
                                emit_event(
                                    app,
                                    role_name,
                                    runtime_key,
                                    app_session_id,
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
                                let _ = app.emit(
                                    "acp/delta",
                                    AcpDeltaPayload {
                                        role: role_name,
                                        runtime_kind: runtime_key,
                                        app_session_id,
                                        delta,
                                    },
                                );
                            }
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
                                        emit_event(
                                            app,
                                            role_name,
                                            runtime_key,
                                            app_session_id,
                                            sequence,
                                            AcpEvent::ThoughtDelta {
                                                text: text.to_string(),
                                            },
                                        );
                                    }
                                }
                                Some("tool_use") => {
                                    emit_event(
                                        app,
                                        role_name,
                                        runtime_key,
                                        app_session_id,
                                        sequence,
                                        AcpEvent::ToolCallUpdate {
                                            tool_call_id: first_string(block, &["id"])
                                                .unwrap_or_else(|| "claude-tool".to_string()),
                                            tool_kind: Some("tool".to_string()),
                                            status: Some("running".to_string()),
                                            title: first_string(block, &["name"]),
                                            content: None,
                                            locations: None,
                                            raw_input: block.get("input").cloned(),
                                            raw_output: None,
                                            terminal_meta: None,
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
                _ => {}
            }
            continue;
        }

        match event_type.as_str() {
            "init" | "initialized" => {
                emit_event(
                    app,
                    role_name,
                    runtime_key,
                    app_session_id,
                    sequence,
                    AcpEvent::StatusUpdate {
                        text: "Antigravity session initialized".to_string(),
                    },
                );
            }
            "step_update" | "step-update" => {
                if let Some(delta) = first_string(payload, &["text_delta", "textDelta"]) {
                    raw_output.push_str(&delta);
                    let _ = app.emit(
                        "acp/delta",
                        AcpDeltaPayload {
                            role: role_name,
                            runtime_kind: runtime_key,
                            app_session_id,
                            delta: &delta,
                        },
                    );
                }
                let step_type =
                    first_string(payload, &["step_type", "stepType"]).unwrap_or_default();
                if step_type != "agent_response" {
                    let tool_id = payload
                        .get("step_index")
                        .or_else(|| payload.get("stepIndex"))
                        .map(Value::to_string)
                        .unwrap_or_else(|| "0".to_string());
                    let tool_info = payload.get("tool_info").or_else(|| payload.get("toolInfo"));
                    emit_event(
                        app,
                        role_name,
                        runtime_key,
                        app_session_id,
                        sequence,
                        AcpEvent::ToolCallUpdate {
                            tool_call_id: format!("agy-step-{tool_id}"),
                            tool_kind: Some("tool".to_string()),
                            status: first_string(payload, &["state", "status"]),
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
                        },
                    );
                }
            }
            "text_delta" | "text-delta" => {
                if let Some(delta) = first_string(payload, &["text", "text_delta", "textDelta"]) {
                    raw_output.push_str(&delta);
                    let _ = app.emit(
                        "acp/delta",
                        AcpDeltaPayload {
                            role: role_name,
                            runtime_kind: runtime_key,
                            app_session_id,
                            delta: &delta,
                        },
                    );
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

pub(super) fn append_cli_config(
    args: &mut Vec<String>,
    protocol: HeadlessProtocol,
    options: &[(String, String)],
    mode: Option<&str>,
) {
    if let Some(model) = find_option(options, &["model", "model_id"]) {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(effort) = find_option(options, &["effort", "reasoning_effort", "reasoningEffort"]) {
        args.extend(["--effort".to_string(), effort]);
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
    sequence: &mut u32,
    event: AcpEvent,
) {
    *sequence = sequence.saturating_add(1);
    let _ = app.emit(
        "acp/stream",
        AcpStreamPayload {
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
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
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
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
    }
}

fn cancelled_result(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
    app: &tauri::AppHandle,
) -> AcpPromptResult {
    error_result(
        runtime_key,
        role_name,
        app_session_id,
        app,
        AcpErrorCode::RequestCancelled,
        "headless prompt cancelled",
        json!({ "mode": "headless-json", "runtimeKey": runtime_key }),
    )
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

fn terminate_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGTERM);
        let _ = libc::kill(pid as i32, libc::SIGTERM);
    }
}
