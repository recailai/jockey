use agent_client_protocol::{self as acp};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::worker::{
    cached_approval, insert_permission, AcpEvent, ConfigStateCell, DeltaSlot, ModeStateCell,
    PendingPermission,
};

const RAW_PAYLOAD_LIMIT: usize = 64 * 1024;
const DEFAULT_TERMINAL_OUTPUT_LIMIT: u64 = 256 * 1024;

fn cap_raw(v: Option<Value>) -> Option<Value> {
    let v = v?;
    let s = v.to_string();
    if s.len() <= RAW_PAYLOAD_LIMIT {
        Some(v)
    } else {
        Some(json!(format!("[truncated {} bytes]", s.len())))
    }
}

fn terminal_meta(meta: Option<&acp::Meta>) -> Option<Value> {
    let meta = meta?;
    let mut out = Map::new();
    if let Some(v) = meta.get("terminal_info") {
        out.insert("terminalInfo".to_string(), v.clone());
    }
    if let Some(v) = meta.get("terminal_output") {
        out.insert("terminalOutput".to_string(), v.clone());
    }
    if let Some(v) = meta.get("terminal_exit") {
        out.insert("terminalExit".to_string(), v.clone());
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

pub(crate) struct TerminalHandle {
    state: Arc<tokio::sync::Mutex<TerminalState>>,
    exit_rx: tokio::sync::watch::Receiver<Option<acp::TerminalExitStatus>>,
    pid: Option<u32>,
}

struct TerminalState {
    output: String,
    /// Total bytes ever written into the terminal (pre-truncation). Let us derive
    /// `truncated = original_bytes_written > output.len()` at response time
    /// instead of storing a redundant bool.
    original_bytes_written: u64,
}

/// Per-client terminal map. Scoped to a single `JockeyUiClient` so the map and
/// the child processes it owns die at the same moment the ACP connection does
/// (reset, reconnect, idle reclaim, process exit). Dropping a client runs
/// `Drop for JockeyUiClient`, which kills every remaining child.
pub(crate) type TerminalMap = Rc<RefCell<HashMap<String, TerminalHandle>>>;

/// Retained for the process-wide shutdown path: if any client instances are
/// still alive when the worker is tearing down, nothing else guarantees their
/// terminals are reaped. This is a safety net — the primary cleanup is
/// Drop-based per client.
pub(super) async fn shutdown_terminals() {
    // Intentionally a no-op today: CONN_MAP.clear() in `shutdown_worker_state`
    // drops every LiveConnection, which drops every client, which drops each
    // per-client terminal map and kills the children. Kept as a stub so
    // callers in `worker/mod.rs` don't have to change their contract.
}

async fn append_terminal_output(
    state: Arc<tokio::sync::Mutex<TerminalState>>,
    output_byte_limit: Option<u64>,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }
    let mut state = state.lock().await;
    let added = String::from_utf8_lossy(bytes);
    state.output.push_str(&added);
    state.original_bytes_written = state
        .original_bytes_written
        .saturating_add(added.len() as u64);
    let Some(limit) = output_byte_limit.map(|limit| limit as usize) else {
        return;
    };
    if state.output.len() <= limit {
        return;
    }
    // Keep the last `limit` bytes. Advance the drain cut forward past the next
    // newline so the surviving prefix always starts at a line boundary — agents
    // get intact lines instead of a mid-token fragment. If the remaining
    // oversize segment has no `\n` at all (single huge line), fall back to the
    // char-boundary cut so we still enforce the byte cap.
    let overflow = state.output.len() - limit;
    let mut drain_end = overflow;
    while drain_end < state.output.len() && !state.output.is_char_boundary(drain_end) {
        drain_end += 1;
    }
    if let Some(nl_offset) = state.output[drain_end..].find('\n') {
        drain_end += nl_offset + 1;
    }
    if drain_end >= state.output.len() {
        // Can't preserve a whole surviving line without emptying the buffer —
        // keep the char-boundary cut so we at least stay under the byte cap.
        drain_end = overflow;
        while drain_end < state.output.len() && !state.output.is_char_boundary(drain_end) {
            drain_end += 1;
        }
    }
    state.output.drain(..drain_end);
}

fn spawn_terminal_drain<R>(
    mut reader: R,
    state: Arc<tokio::sync::Mutex<TerminalState>>,
    output_byte_limit: Option<u64>,
) where
    R: tokio::io::AsyncRead + Unpin + 'static,
{
    tokio::task::spawn_local(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => append_terminal_output(state.clone(), output_byte_limit, &buf[..n]).await,
            }
        }
    });
}

#[cfg(unix)]
fn terminate_terminal_process(pid: Option<u32>) {
    if let Some(pid) = pid {
        unsafe {
            let pgid = -(pid as i32);
            let _ = libc::kill(pgid, libc::SIGTERM);
            let _ = libc::kill(pid as i32, libc::SIGTERM);
            let _ = libc::kill(pgid, libc::SIGKILL);
            let _ = libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn terminate_terminal_process(_pid: Option<u32>) {}

pub(super) struct JockeyUiClient {
    pub(super) delta_slot: DeltaSlot,
    pub(super) auto_approve: bool,
    pub(super) runtime_key: String,
    pub(super) role_name: String,
    pub(super) app_session_id: String,
    pub(super) expected_session_id: Arc<Mutex<Option<acp::SessionId>>>,
    /// Shared with the owning `LiveConnection`; writebacks from session
    /// notifications land here without a worker round-trip.
    pub(super) mode_state: ModeStateCell,
    pub(super) config_state: ConfigStateCell,
    /// Terminals owned by this client. Dropped (and reaped) when the client
    /// drops — which happens when its owning `LiveConnection` is evicted.
    pub(super) terminals: TerminalMap,
}

impl Drop for JockeyUiClient {
    fn drop(&mut self) {
        // Kill every child we still own. Intentionally blocking: we are on the
        // worker LocalSet thread (enforced by !Send of TerminalMap) and these
        // are non-blocking signal sends.
        if let Ok(mut map) = self.terminals.try_borrow_mut() {
            for (_, handle) in map.drain() {
                terminate_terminal_process(handle.pid);
            }
        }
    }
}

impl JockeyUiClient {
    fn validate_session(&self, received: &acp::SessionId) -> acp::Result<()> {
        let expected = self
            .expected_session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(expected) = expected {
            if expected != *received {
                use super::adapter::acp_log;
                acp_log(
                    "session.request_mismatch",
                    json!({
                        "runtime": self.runtime_key,
                        "role": self.role_name,
                        "appSession": self.app_session_id,
                        "expected": expected.to_string(),
                        "received": received.to_string()
                    }),
                );
                return Err(acp::Error::new(
                    acp::ErrorCode::InvalidParams.into(),
                    "request session does not match active ACP session",
                ));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for JockeyUiClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        self.validate_session(&args.session_id)?;
        if self.auto_approve {
            let option = args
                .options
                .iter()
                .find(|o| {
                    matches!(
                        o.kind,
                        acp::PermissionOptionKind::AllowOnce
                            | acp::PermissionOptionKind::AllowAlways
                    )
                })
                .or_else(|| args.options.first());
            let outcome = if let Some(opt) = option {
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    opt.option_id.clone(),
                ))
            } else {
                acp::RequestPermissionOutcome::Cancelled
            };
            return Ok(acp::RequestPermissionResponse::new(outcome));
        }

        let cache_key = permission_cache_key(&args);
        if let Some(option_id) = cached_approval(&cache_key) {
            return Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    acp::PermissionOptionId::from(option_id),
                )),
            ));
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let event = AcpEvent::PermissionRequest {
            request_id: request_id.clone(),
            title: args.tool_call.fields.title.clone().unwrap_or_default(),
            description: None,
            options: args
                .options
                .iter()
                .map(|o| serde_json::to_value(o).unwrap_or(json!({})))
                .collect(),
        };
        let permission_delta_tx = self
            .delta_slot
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        if let Some(tx) = permission_delta_tx.as_ref() {
            if tx.try_send(event).is_err() {
                use super::adapter::acp_log;
                acp_log(
                    "permission.channel.drop",
                    serde_json::json!({ "requestId": request_id }),
                );
            }
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let allow_always_option_ids = args
            .options
            .iter()
            .filter(|o| matches!(o.kind, acp::PermissionOptionKind::AllowAlways))
            .map(|o| o.option_id.to_string())
            .collect();
        insert_permission(
            request_id.clone(),
            PendingPermission {
                runtime_key: self.runtime_key.clone(),
                role_name: self.role_name.clone(),
                app_session_id: self.app_session_id.clone(),
                cache_key,
                allow_always_option_ids,
                delta_tx: permission_delta_tx,
                tx,
            },
        );
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(decision)) => Ok(acp::RequestPermissionResponse::new(decision)),
            _ => {
                super::worker::permission_requests().remove(&request_id);
                // Notify frontend that this permission request timed out / was cancelled
                if let Ok(guard) = self.delta_slot.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.try_send(AcpEvent::PermissionExpired {
                            request_id: request_id.clone(),
                        });
                    }
                }
                Ok(acp::RequestPermissionResponse::new(
                    acp::RequestPermissionOutcome::Cancelled,
                ))
            }
        }
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        self.validate_session(&args.session_id)?;
        let event = match args.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                let text = match chunk.content {
                    acp::ContentBlock::Text(tc) => tc.text,
                    acp::ContentBlock::ResourceLink(rl) => rl.uri,
                    _ => return Ok(()),
                };
                if text.is_empty() {
                    return Ok(());
                }
                AcpEvent::TextDelta { text }
            }
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = match chunk.content {
                    acp::ContentBlock::Text(tc) => tc.text,
                    _ => return Ok(()),
                };
                if text.is_empty() {
                    return Ok(());
                }
                AcpEvent::ThoughtDelta { text }
            }
            acp::SessionUpdate::ToolCall(tc) => AcpEvent::ToolCall {
                tool_call_id: tc.tool_call_id.to_string(),
                title: tc.title.clone(),
                tool_kind: serde_json::to_value(&tc.kind)
                    .and_then(|v| Ok(v.as_str().unwrap_or("unknown").to_string()))
                    .unwrap_or_else(|_| "unknown".to_string()),
                status: serde_json::to_value(&tc.status)
                    .and_then(|v| Ok(v.as_str().unwrap_or("pending").to_string()))
                    .unwrap_or_else(|_| "pending".to_string()),
                content: if tc.content.is_empty() {
                    None
                } else {
                    Some(
                        tc.content
                            .iter()
                            .map(|c| serde_json::to_value(c).unwrap_or(json!({})))
                            .collect(),
                    )
                },
                locations: if tc.locations.is_empty() {
                    None
                } else {
                    Some(
                        tc.locations
                            .iter()
                            .map(|loc| serde_json::to_value(loc).unwrap_or(json!({})))
                            .collect(),
                    )
                },
                raw_input: cap_raw(tc.raw_input.clone()),
                raw_output: cap_raw(tc.raw_output.clone()),
                terminal_meta: terminal_meta(tc.meta.as_ref()),
            },
            acp::SessionUpdate::ToolCallUpdate(tcu) => AcpEvent::ToolCallUpdate {
                tool_call_id: tcu.tool_call_id.to_string(),
                tool_kind: tcu.fields.kind.map(|k| {
                    serde_json::to_value(&k)
                        .and_then(|v| Ok(v.as_str().unwrap_or("").to_string()))
                        .unwrap_or_default()
                }),
                status: tcu.fields.status.map(|s| {
                    serde_json::to_value(&s)
                        .and_then(|v| Ok(v.as_str().unwrap_or("").to_string()))
                        .unwrap_or_default()
                }),
                title: tcu.fields.title.clone(),
                content: tcu.fields.content.as_ref().map(|blocks| {
                    blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or(json!({})))
                        .collect()
                }),
                locations: tcu.fields.locations.as_ref().map(|items| {
                    items
                        .iter()
                        .map(|loc| serde_json::to_value(loc).unwrap_or(json!({})))
                        .collect()
                }),
                raw_input: cap_raw(tcu.fields.raw_input.clone()),
                raw_output: cap_raw(tcu.fields.raw_output.clone()),
                terminal_meta: terminal_meta(tcu.meta.as_ref()),
            },
            acp::SessionUpdate::Plan(plan) => AcpEvent::Plan {
                entries: plan
                    .entries
                    .iter()
                    .map(|e| serde_json::to_value(e).unwrap_or(json!({})))
                    .collect(),
            },
            acp::SessionUpdate::CurrentModeUpdate(mode) => {
                if let Ok(mut cell) = self.mode_state.try_borrow_mut() {
                    if let Some(state) = cell.as_mut() {
                        state.current_mode_id = mode.current_mode_id.clone();
                    }
                }
                AcpEvent::ModeUpdate {
                    mode_id: mode.current_mode_id.to_string(),
                }
            }
            acp::SessionUpdate::ConfigOptionUpdate(cfg) => {
                if let Ok(mut cell) = self.config_state.try_borrow_mut() {
                    *cell = cfg.config_options.clone();
                }
                AcpEvent::ConfigUpdate {
                    options: cfg
                        .config_options
                        .iter()
                        .map(|o| serde_json::to_value(o).unwrap_or(json!({})))
                        .collect(),
                }
            }
            acp::SessionUpdate::SessionInfoUpdate(info) => AcpEvent::SessionInfo {
                title: match info.title {
                    acp::MaybeUndefined::Value(v) => Some(v),
                    _ => None,
                },
            },
            acp::SessionUpdate::AvailableCommandsUpdate(cmds) => AcpEvent::AvailableCommands {
                commands: cmds
                    .available_commands
                    .iter()
                    .map(|c| serde_json::to_value(c).unwrap_or(json!({})))
                    .collect(),
            },
            _ => return Ok(()),
        };
        let tx = self
            .delta_slot
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        if let Some(tx) = tx {
            use super::adapter::acp_log;
            // Backpressure: prefer fast non-blocking path; only fall back to
            // awaited send when the channel is momentarily full. Closed channel
            // means the consumer (execute loop) is gone — drop silently.
            match tx.try_send(event) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                    let cap = super::worker::DELTA_CHANNEL_CAPACITY;
                    let used = cap.saturating_sub(tx.capacity());
                    acp_log(
                        "delta.channel.backpressure",
                        serde_json::json!({ "capacity": cap, "queued": used }),
                    );
                    // Block the agent's session_notification handler until the
                    // execute loop drains. This is what we want: the agent
                    // (single-threaded) will pause emitting further updates,
                    // which prevents lossy UI state.
                    if tx.send(event).await.is_err() {
                        acp_log("delta.channel.closed", serde_json::json!({}));
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    acp_log("delta.channel.closed", serde_json::json!({}));
                }
            }
        }
        Ok(())
    }

    async fn read_text_file(
        &self,
        args: acp::ReadTextFileRequest,
    ) -> acp::Result<acp::ReadTextFileResponse> {
        self.validate_session(&args.session_id)?;
        let path = args.path;
        let full = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;

        // Apply optional line/limit slicing (1-based per spec)
        let content = match (args.line, args.limit) {
            (None, None) => full,
            (start, limit) => {
                let start_idx = start.map(|l| l.saturating_sub(1) as usize).unwrap_or(0);
                let lines: Vec<&str> = full.lines().collect();
                let slice = &lines[start_idx.min(lines.len())..];
                let slice = if let Some(n) = limit {
                    &slice[..n.min(slice.len() as u32) as usize]
                } else {
                    slice
                };
                slice.join("\n")
            }
        };

        Ok(acp::ReadTextFileResponse::new(content))
    }

    async fn write_text_file(
        &self,
        args: acp::WriteTextFileRequest,
    ) -> acp::Result<acp::WriteTextFileResponse> {
        self.validate_session(&args.session_id)?;
        if let Some(parent) = args.path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        tokio::fs::write(&args.path, &args.content)
            .await
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        crate::git::notify_changed(&args.path);
        Ok(acp::WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        args: acp::CreateTerminalRequest,
    ) -> acp::Result<acp::CreateTerminalResponse> {
        self.validate_session(&args.session_id)?;
        let mut cmd = tokio::process::Command::new(&args.command);
        cmd.args(&args.args);
        if let Some(cwd) = &args.cwd {
            cmd.current_dir(cwd);
        }
        for env_var in &args.env {
            cmd.env(&env_var.name, &env_var.value);
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        let terminal_id = acp::TerminalId::from(uuid::Uuid::new_v4().to_string());
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let state = Arc::new(tokio::sync::Mutex::new(TerminalState {
            output: String::new(),
            original_bytes_written: 0,
        }));
        let output_byte_limit = args
            .output_byte_limit
            .or(Some(DEFAULT_TERMINAL_OUTPUT_LIMIT));
        if let Some(stdout) = stdout {
            spawn_terminal_drain(stdout, state.clone(), output_byte_limit);
        }
        if let Some(stderr) = stderr {
            spawn_terminal_drain(stderr, state.clone(), output_byte_limit);
        }
        let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
        tokio::task::spawn_local(async move {
            let status = child.wait().await.ok();
            let exit_status = status
                .map(|s| acp::TerminalExitStatus::new().exit_code(s.code().map(|c| c as u32)));
            let _ = exit_tx.send(exit_status);
        });
        self.terminals.borrow_mut().insert(
            terminal_id.to_string(),
            TerminalHandle {
                state,
                exit_rx,
                pid,
            },
        );
        Ok(acp::CreateTerminalResponse::new(terminal_id))
    }

    async fn terminal_output(
        &self,
        args: acp::TerminalOutputRequest,
    ) -> acp::Result<acp::TerminalOutputResponse> {
        self.validate_session(&args.session_id)?;
        let key = args.terminal_id.to_string();
        let (state_arc, exit_rx) = {
            let map = self.terminals.borrow();
            let handle = map.get(&key).ok_or_else(|| {
                acp::Error::new(acp::ErrorCode::InvalidParams.into(), "terminal not found")
            })?;
            (handle.state.clone(), handle.exit_rx.clone())
        };
        let state = state_arc.lock().await;
        let output = state.output.clone();
        let truncated = state.original_bytes_written > output.len() as u64;
        let exit_status = exit_rx.borrow().clone();
        Ok(acp::TerminalOutputResponse::new(output, truncated).exit_status(exit_status))
    }

    async fn wait_for_terminal_exit(
        &self,
        args: acp::WaitForTerminalExitRequest,
    ) -> acp::Result<acp::WaitForTerminalExitResponse> {
        self.validate_session(&args.session_id)?;
        let key = args.terminal_id.to_string();
        let mut exit_rx = {
            let map = self.terminals.borrow();
            let handle = map.get(&key).ok_or_else(|| {
                acp::Error::new(acp::ErrorCode::InvalidParams.into(), "terminal not found")
            })?;
            handle.exit_rx.clone()
        };
        loop {
            if let Some(status) = exit_rx.borrow().clone() {
                return Ok(acp::WaitForTerminalExitResponse::new(status));
            }
            if exit_rx.changed().await.is_err() {
                return Err(acp::Error::new(
                    acp::ErrorCode::InternalError.into(),
                    "terminal exit watcher closed",
                ));
            }
        }
    }

    async fn kill_terminal(
        &self,
        args: acp::KillTerminalRequest,
    ) -> acp::Result<acp::KillTerminalResponse> {
        self.validate_session(&args.session_id)?;
        let key = args.terminal_id.to_string();
        let pid = self.terminals.borrow().get(&key).and_then(|h| h.pid);
        terminate_terminal_process(pid);
        Ok(acp::KillTerminalResponse::new())
    }

    async fn release_terminal(
        &self,
        args: acp::ReleaseTerminalRequest,
    ) -> acp::Result<acp::ReleaseTerminalResponse> {
        self.validate_session(&args.session_id)?;
        let key = args.terminal_id.to_string();
        if let Some(handle) = self.terminals.borrow_mut().remove(&key) {
            terminate_terminal_process(handle.pid);
        }
        Ok(acp::ReleaseTerminalResponse::new())
    }
}

fn permission_cache_key(args: &acp::RequestPermissionRequest) -> String {
    let title = args.tool_call.fields.title.clone().unwrap_or_default();
    let kind = args
        .tool_call
        .fields
        .kind
        .as_ref()
        .and_then(|k| serde_json::to_value(k).ok())
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let locations = args
        .tool_call
        .fields
        .locations
        .as_ref()
        .map(|items| {
            items
                .iter()
                .filter_map(|loc| serde_json::to_value(loc).ok())
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    let raw = args
        .tool_call
        .fields
        .raw_input
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    format!("{kind}\n{title}\n{locations}\n{raw}")
}
