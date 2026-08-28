use serde_json::json;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use super::super::super::adapter::{acp_log, clip, HeadlessProtocol};
use super::super::super::error::{push_stderr_tail, stderr_tail};
use super::super::super::protocol as acp;
use super::super::super::worker::{
    register_child_pid, unregister_child_pid, AcpEvent, AcpPromptResult,
};
use super::{
    append_cli_config, append_stderr, cancelled_result, classify_error, compose_prompt, emit_event,
    error_result, headless_active, headless_cancelled, headless_children, headless_key,
    headless_locks, protocol_display_name, protocol_transport_name, read_headless_turn,
    ActivityGuard, HeadlessTurn,
};

const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
const STREAM_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const STREAM_IDLE_AFTER: Duration = Duration::from_secs(300);

static STREAM_SESSIONS: OnceLock<dashmap::DashMap<String, Arc<Mutex<Option<StreamSession>>>>> =
    OnceLock::new();

fn stream_sessions() -> &'static dashmap::DashMap<String, Arc<Mutex<Option<StreamSession>>>> {
    STREAM_SESSIONS.get_or_init(dashmap::DashMap::new)
}

struct StreamProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<std::sync::Mutex<String>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl StreamProcess {
    fn spawn(
        protocol: HeadlessProtocol,
        binary: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
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
        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to start {} stream-json: {error}",
                protocol_display_name(protocol)
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            format!(
                "{} stream-json stdin pipe unavailable",
                protocol_display_name(protocol)
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            format!(
                "{} stream-json stdout pipe unavailable",
                protocol_display_name(protocol)
            )
        })?;
        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_task = child.stderr.take().map(|stream| {
            let stderr = stderr.clone();
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
                                    "headless.stderr",
                                    json!({ "binary": protocol_display_name(protocol), "line": clip(line.trim(), 360) }),
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
        })
    }

    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    async fn send_prompt(
        &mut self,
        protocol: HeadlessProtocol,
        prompt: &str,
    ) -> Result<(), String> {
        let frame_value = match protocol {
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
        };
        let mut frame = serde_json::to_vec(&frame_value).map_err(|error| {
            format!(
                "{} stream input serialization failed: {error}",
                protocol_display_name(protocol)
            )
        })?;
        frame.push(b'\n');
        self.stdin.write_all(&frame).await.map_err(|error| {
            format!(
                "failed to send prompt to {}: {error}",
                protocol_display_name(protocol)
            )
        })?;
        self.stdin.flush().await.map_err(|error| {
            format!(
                "failed to flush prompt to {}: {error}",
                protocol_display_name(protocol)
            )
        })
    }

    async fn read_turn(
        &mut self,
        protocol: HeadlessProtocol,
        app: &tauri::AppHandle,
        role_name: &str,
        runtime_key: &'static str,
        app_session_id: &str,
        sequence: &mut u32,
    ) -> HeadlessTurn {
        read_headless_turn(
            &mut self.stdout,
            protocol,
            true,
            app,
            role_name,
            runtime_key,
            app_session_id,
            sequence,
        )
        .await
    }

    fn stderr_tail(&self) -> String {
        stderr_tail(&self.stderr)
    }

    async fn close(&mut self) {
        let _ = self.stdin.shutdown().await;
        if let Some(pid) = self.pid() {
            super::terminate_pid(pid);
        }
        let _ = self.child.kill().await;
        let _ = tokio::time::timeout(STREAM_CLOSE_TIMEOUT, self.child.wait()).await;
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await;
        }
    }
}

struct StreamSession {
    process: StreamProcess,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
    conversation_id: Option<String>,
    last_used: Instant,
}

impl StreamSession {
    fn matches(
        &mut self,
        binary: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
        resume_conversation_id: Option<&str>,
    ) -> bool {
        let args_match = args_without_conversation(&self.args) == args_without_conversation(args);
        let conversation_match = match resume_conversation_id {
            None => true,
            Some(expected) => self.conversation_id.as_deref() == Some(expected),
        };
        let matches = self.binary == binary
            && args_match
            && self.env.as_slice() == env
            && self.cwd == cwd
            && conversation_match
            && self.process.is_alive();
        if matches {
            self.last_used = Instant::now();
        }
        matches
    }
}

pub(super) async fn execute_stream_runtime(
    protocol: HeadlessProtocol,
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
    conversation: bool,
    resume_session_id: Option<String>,
    mcp_servers: &[acp::McpServer],
    app_session_id: &str,
) -> AcpPromptResult {
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

    let mut args = adapter_args.to_vec();
    append_cli_config(
        &mut args,
        protocol,
        &role_config_options,
        role_mode.as_deref(),
    );
    if auto_approve {
        if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
            if let Some(config_path) =
                crate::acp::session::perm_bridge::claude_mcp_config_path(&key, mcp_servers)
            {
                args.push("--mcp-config".to_string());
                args.push(config_path);
            }
        }
        args.push("--dangerously-skip-permissions".to_string());
    } else if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
        // Permission-gated native Claude: wire prompts into the Jockey UI via
        // the permission MCP bridge. Never fall back to a silent bypass.
        match crate::acp::session::perm_bridge::claude_permission_args(&key, mcp_servers) {
            Ok(permission_args) => args.extend(permission_args),
            Err(error) => {
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    app,
                    super::super::super::error::AcpErrorCode::ConnectionFailed,
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
        if let Some(session_id) = resume_session_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        {
            args.extend([
                match protocol {
                    HeadlessProtocol::AgyStreamJson => "--conversation".to_string(),
                    HeadlessProtocol::ClaudeStreamJson => "--resume".to_string(),
                },
                session_id.to_string(),
            ]);
        }
    }
    args.extend([
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ]);

    let slot = stream_sessions()
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(None)))
        .clone();
    let mut slot = slot.lock().await;
    let reused = slot.as_mut().is_some_and(|session| {
        session.matches(binary, &args, env, cwd, resume_session_id.as_deref())
    });
    if !reused {
        close_session(&mut slot, &key).await;
        let process = match StreamProcess::spawn(protocol, binary, &args, env, cwd) {
            Ok(process) => process,
            Err(error) => {
                return error_result(
                    runtime_key,
                    role_name,
                    app_session_id,
                    app,
                    super::super::super::error::AcpErrorCode::ProcessCrashed,
                    error,
                    json!({
                        "mode": "headless-json",
                        "transport": protocol_transport_name(protocol),
                        "runtimeKey": runtime_key,
                    }),
                )
            }
        };
        if let Some(pid) = process.pid() {
            headless_children().insert(key.clone(), pid);
            register_child_pid(pid);
        }
        *slot = Some(StreamSession {
            process,
            binary: binary.to_string(),
            args,
            env: env.to_vec(),
            cwd: cwd.to_string(),
            conversation_id: None,
            last_used: Instant::now(),
        });
    }

    if headless_cancelled().contains(&key) {
        close_session(&mut slot, &key).await;
        stream_sessions().remove(&key);
        return cancelled_result(runtime_key, role_name, app_session_id, app);
    }

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
            text: if reused {
                format!("Reusing {} session...", protocol_display_name(protocol))
            } else {
                format!("Connecting to {}...", protocol_display_name(protocol))
            },
        },
    );

    let input = compose_prompt(prompt, context);
    let send_result = {
        let session = slot
            .as_mut()
            .expect("headless stream session must exist after spawn or reuse");
        session.process.send_prompt(protocol, &input).await
    };
    if let Err(error) = send_result {
        close_session(&mut slot, &key).await;
        stream_sessions().remove(&key);
        return error_result(
            runtime_key,
            role_name,
            app_session_id,
            app,
            super::super::super::error::AcpErrorCode::ConnectionFailed,
            error,
            json!({
                "mode": "headless-json",
                "transport": protocol_transport_name(protocol),
                "runtimeKey": runtime_key,
            }),
        );
    }

    let read_result = {
        let session = slot
            .as_mut()
            .expect("headless stream session must exist before reading");
        tokio::time::timeout(
            STREAM_TIMEOUT,
            session.process.read_turn(
                protocol,
                app,
                role_name,
                runtime_key,
                app_session_id,
                &mut sequence,
            ),
        )
        .await
    };
    let turn = match read_result {
        Ok(result) => result,
        Err(_) => {
            close_session(&mut slot, &key).await;
            stream_sessions().remove(&key);
            return error_result(
                runtime_key,
                role_name,
                app_session_id,
                app,
                super::super::super::error::AcpErrorCode::PromptTimeout,
                format!(
                    "{} did not finish within {}s",
                    protocol_display_name(protocol),
                    STREAM_TIMEOUT.as_secs()
                ),
                json!({
                    "mode": "headless-json",
                    "transport": protocol_transport_name(protocol),
                    "runtimeKey": runtime_key,
                }),
            );
        }
    };

    let stderr = slot
        .as_ref()
        .map(|session| session.process.stderr_tail())
        .unwrap_or_default();
    let process_alive = slot
        .as_mut()
        .is_some_and(|session| session.process.is_alive());
    let resolved_conversation_id = turn.conversation_id.clone().or_else(|| {
        slot.as_ref()
            .and_then(|session| session.conversation_id.clone())
    });
    if let Some(id) = resolved_conversation_id
        .as_deref()
        .filter(|id| !id.is_empty())
    {
        if let Some(session) = slot.as_mut() {
            session.conversation_id = Some(id.to_string());
        }
    }
    if let Some(session) = slot.as_mut() {
        session.last_used = Instant::now();
    }

    let cancelled = headless_cancelled().contains(&key);
    if cancelled || turn.error.is_some() || !turn.result_seen || !process_alive {
        close_session(&mut slot, &key).await;
        stream_sessions().remove(&key);
    }
    if cancelled {
        return cancelled_result(runtime_key, role_name, app_session_id, app);
    }
    if let Some(error) = turn.error {
        return error_result(
            runtime_key,
            role_name,
            app_session_id,
            app,
            classify_error(&error),
            append_stderr(error, &stderr),
            json!({
                "mode": "headless-json",
                "transport": protocol_transport_name(protocol),
                "runtimeKey": runtime_key,
                "conversationId": resolved_conversation_id,
                "status": turn.status,
            }),
        );
    }
    if !turn.result_seen || !process_alive {
        return error_result(
            runtime_key,
            role_name,
            app_session_id,
            app,
            super::super::super::error::AcpErrorCode::ProcessCrashed,
            append_stderr(
                format!(
                    "{} stream-json process exited before result",
                    protocol_display_name(protocol)
                ),
                &stderr,
            ),
            json!({
                "mode": "headless-json",
                "transport": protocol_transport_name(protocol),
                "runtimeKey": runtime_key,
                "conversationId": resolved_conversation_id,
                "status": turn.status,
            }),
        );
    }

    let output = turn.response.unwrap_or(turn.output);
    acp_log(
        "headless.stream.execute.ok",
        json!({
            "runtime": runtime_key,
            "role": role_name,
            "outputSize": output.len(),
            "conversationId": resolved_conversation_id,
            "status": turn.status,
            "mcpServerCount": mcp_servers.len(),
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
            "conversationId": resolved_conversation_id,
            "status": turn.status,
            "persistent": true,
        }),
    }
}

pub(super) async fn discard_session(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    let key = headless_key(runtime_key, role_name, app_session_id);
    let _ = super::cancel_headless(runtime_key, role_name, app_session_id);
    let lock = headless_locks()
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    if let Some((_, slot)) = stream_sessions().remove(&key) {
        let mut slot = slot.lock().await;
        close_session(&mut slot, &key).await;
    }
    headless_active().remove(&key);
    headless_cancelled().remove(&key);
    super::super::super::runtime_state::clear_session(app_session_id, runtime_key, role_name);
    Ok(())
}

pub(super) async fn shutdown_sessions() {
    let keys = stream_sessions()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for key in keys {
        let Some(slot) = stream_sessions().get(&key).map(|entry| entry.clone()) else {
            continue;
        };
        headless_cancelled().insert(key.clone());
        if let Some(pid) = headless_children().get(&key).map(|entry| *entry) {
            super::terminate_pid(pid);
        }
        let mut slot = slot.lock().await;
        close_session(&mut slot, &key).await;
        stream_sessions().remove(&key);
        headless_active().remove(&key);
        headless_cancelled().remove(&key);
    }
}

pub(super) fn reclaim_idle_sessions() {
    let now = Instant::now();
    let keys = stream_sessions()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for key in keys {
        if headless_active().contains_key(&key) {
            continue;
        }
        let Some(slot) = stream_sessions().get(&key).map(|entry| entry.clone()) else {
            continue;
        };
        let Ok(mut slot) = slot.try_lock() else {
            continue;
        };
        let expired = slot
            .as_ref()
            .is_some_and(|session| now.duration_since(session.last_used) >= STREAM_IDLE_AFTER);
        if !expired {
            continue;
        }
        if let Some(session) = slot.take() {
            if let Some(pid) = session.process.pid() {
                super::terminate_pid(pid);
                cleanup_pid(&key, Some(pid));
            }
        }
        crate::acp::session::perm_bridge::cancel_permissions_for_key(&key);
        stream_sessions().remove(&key);
        acp_log("headless.stream.idle_reclaim", json!({ "key": key }));
    }
}

async fn close_session(slot: &mut Option<StreamSession>, key: &str) {
    let Some(mut session) = slot.take() else {
        return;
    };
    let pid = session.process.pid();
    session.process.close().await;
    cleanup_pid(key, pid);
    crate::acp::session::perm_bridge::cancel_permissions_for_key(key);
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

fn args_without_conversation(args: &[String]) -> Vec<&str> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--conversation" || arg == "--resume" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--conversation=") || arg.starts_with("--resume=") {
            continue;
        }
        filtered.push(arg.as_str());
    }
    filtered
}
