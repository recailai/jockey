use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;
use tokio::sync::{oneshot, Mutex};

use super::super::adapter::{acp_log, NativeProtocol};
use super::super::error::AcpErrorCode;
use super::super::protocol as acp;
use super::super::runtime_state::{
    clear_runtime, fresh_config_options, list_discovered_modes, remember_runtime_config_options,
    remember_runtime_modes,
};
use super::super::worker::{register_child_pid, unregister_child_pid, AcpEvent, AcpPromptResult};
use super::execute::{AcpDeltaPayload, AcpStreamPayload};

mod approval;
mod codex;
pub(crate) mod codex_admin;
mod pi;
mod process;
use process::{terminate_pid, transport_name, NativeProcess};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
const NATIVE_IDLE_AFTER: Duration = Duration::from_secs(300);
const NATIVE_DELTA_BATCH_BYTES: usize = 4096;
const NATIVE_DELTA_FLUSH_AFTER: Duration = Duration::from_millis(30);

static NATIVE_LOCKS: OnceLock<dashmap::DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();
static NATIVE_SESSIONS: OnceLock<dashmap::DashMap<String, Arc<Mutex<Option<NativeSession>>>>> =
    OnceLock::new();
static NATIVE_CHILDREN: OnceLock<dashmap::DashMap<String, u32>> = OnceLock::new();
static NATIVE_ACTIVE: OnceLock<dashmap::DashSet<String>> = OnceLock::new();
static NATIVE_CANCELLED: OnceLock<dashmap::DashSet<String>> = OnceLock::new();
static NATIVE_WRITERS: OnceLock<dashmap::DashMap<String, Arc<Mutex<tokio::process::ChildStdin>>>> =
    OnceLock::new();
static NATIVE_TURNS: OnceLock<dashmap::DashMap<String, ActiveNativeTurn>> = OnceLock::new();
static NATIVE_CONTROL_ACKS: OnceLock<
    dashmap::DashMap<String, oneshot::Sender<Result<(), String>>>,
> = OnceLock::new();

fn native_locks() -> &'static dashmap::DashMap<String, Arc<Mutex<()>>> {
    NATIVE_LOCKS.get_or_init(dashmap::DashMap::new)
}

fn native_sessions() -> &'static dashmap::DashMap<String, Arc<Mutex<Option<NativeSession>>>> {
    NATIVE_SESSIONS.get_or_init(dashmap::DashMap::new)
}

fn native_children() -> &'static dashmap::DashMap<String, u32> {
    NATIVE_CHILDREN.get_or_init(dashmap::DashMap::new)
}

fn native_active() -> &'static dashmap::DashSet<String> {
    NATIVE_ACTIVE.get_or_init(dashmap::DashSet::new)
}

fn native_cancelled() -> &'static dashmap::DashSet<String> {
    NATIVE_CANCELLED.get_or_init(dashmap::DashSet::new)
}

fn native_writers() -> &'static dashmap::DashMap<String, Arc<Mutex<tokio::process::ChildStdin>>> {
    NATIVE_WRITERS.get_or_init(dashmap::DashMap::new)
}

fn native_turns() -> &'static dashmap::DashMap<String, ActiveNativeTurn> {
    NATIVE_TURNS.get_or_init(dashmap::DashMap::new)
}

fn native_control_acks() -> &'static dashmap::DashMap<String, oneshot::Sender<Result<(), String>>> {
    NATIVE_CONTROL_ACKS.get_or_init(dashmap::DashMap::new)
}

fn native_key(runtime_key: &str, role_name: &str, app_session_id: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

#[derive(Clone)]
struct ActiveNativeTurn {
    protocol: NativeProtocol,
    thread_id: Option<String>,
    turn_id: Option<String>,
}

pub(super) fn set_active_native_turn(
    key: &str,
    protocol: NativeProtocol,
    thread_id: Option<String>,
    turn_id: Option<String>,
) {
    native_turns().insert(
        key.to_string(),
        ActiveNativeTurn {
            protocol,
            thread_id,
            turn_id,
        },
    );
}

fn clear_active_native_turn(key: &str) {
    native_turns().remove(key);
}

async fn write_native_frame(
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    frame: Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("native control serialization failed: {error}"))?;
    bytes.push(b'\n');
    let mut stdin = writer.lock().await;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|error| format!("native control write failed: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("native control flush failed: {error}"))
}

pub(super) fn resolve_native_control_response(control_key: &str, message: &Value) -> bool {
    let Some((_, sender)) = native_control_acks().remove(control_key) else {
        return false;
    };
    let result = if message.get("error").is_some()
        || message.get("success").and_then(Value::as_bool) == Some(false)
    {
        let error = message
            .get("error")
            .or_else(|| message.get("message"))
            .map(Value::to_string)
            .unwrap_or_else(|| "native control request rejected".to_string());
        Err(error)
    } else {
        Ok(())
    };
    let _ = sender.send(result);
    true
}

pub(super) async fn steer_native(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
    prompt: &str,
) -> Result<(), String> {
    let key = native_key(runtime_key, role_name, app_session_id);
    if !native_active().contains(&key) {
        return Err("no active native turn to steer".to_string());
    }
    let active = native_turns()
        .get(&key)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| "native turn control is not ready".to_string())?;
    let (control_key, frame) = match active.protocol {
        NativeProtocol::PiRpc => (
            key.clone(),
            json!({
                "type": "steer",
                "message": prompt,
                "images": [],
            }),
        ),
        NativeProtocol::CodexAppServer => {
            let thread_id = active
                .thread_id
                .as_deref()
                .ok_or_else(|| "Codex thread id is not ready for steer".to_string())?;
            let turn_id = active
                .turn_id
                .as_deref()
                .ok_or_else(|| "Codex turn id is not ready for steer".to_string())?;
            let control_id = format!("jockey-steer-{}", uuid::Uuid::new_v4());
            (
                control_id.clone(),
                json!({
                    "id": control_id,
                    "method": "turn/steer",
                    "params": {
                        "threadId": thread_id,
                        "input": [{ "type": "text", "text": prompt }],
                        "expectedTurnId": turn_id,
                    },
                }),
            )
        }
    };
    let writer = native_writers()
        .get(&key)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| "native process writer is unavailable".to_string())?;
    let (ack_tx, ack_rx) = oneshot::channel();
    native_control_acks().insert(control_key.clone(), ack_tx);
    if let Err(error) = write_native_frame(writer, frame).await {
        native_control_acks().remove(&control_key);
        return Err(error);
    }
    match tokio::time::timeout(Duration::from_secs(5), ack_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("native control acknowledgement was dropped".to_string()),
        Err(_) => {
            native_control_acks().remove(&control_key);
            Err("native control acknowledgement timed out".to_string())
        }
    }
}

pub(super) fn cancel_native(runtime_key: &str, role_name: &str, app_session_id: &str) -> bool {
    let key = native_key(runtime_key, role_name, app_session_id);
    if !native_active().contains(&key) {
        return false;
    }
    native_cancelled().insert(key.clone());
    let is_pi = runtime_key.starts_with("pi") || runtime_key.contains("pi-");
    if is_pi {
        if let Some(writer) = native_writers()
            .get(&key)
            .map(|entry| entry.value().clone())
        {
            tokio::spawn(async move {
                let _ = write_native_frame(writer, serde_json::json!({ "type": "abort" })).await;
            });
        }
        acp_log(
            "native.cancel.abort_sent",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
    } else if let Some(pid) = native_children().get(&key).map(|entry| *entry) {
        terminate_pid(pid);
        acp_log(
            "native.cancel",
            json!({ "runtime": runtime_key, "role": role_name, "pid": pid }),
        );
    }
    true
}

pub(super) async fn cancel_native_and_wait(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    if !cancel_native(runtime_key, role_name, app_session_id) {
        return Ok(());
    }
    let key = native_key(runtime_key, role_name, app_session_id);
    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        while native_active().contains(&key) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match drained {
        Ok(()) => {
            native_cancelled().remove(&key);
            Ok(())
        }
        Err(_) => Err(format!("timed out waiting for {runtime_key} turn to stop")),
    }
}

pub(super) async fn discard_native_session(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    let key = native_key(runtime_key, role_name, app_session_id);
    let _ = cancel_native(runtime_key, role_name, app_session_id);
    let lock = native_locks()
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    if let Some((_, slot)) = native_sessions().remove(&key) {
        let mut slot = slot.lock().await;
        close_native_session(&mut slot, &key).await;
    }
    native_active().remove(&key);
    native_cancelled().remove(&key);
    super::super::runtime_state::clear_session(app_session_id, runtime_key, role_name);
    Ok(())
}

/// Roll back the live codex thread owned by the session slot by `num_turns`.
pub(crate) async fn rollback_live_codex_thread(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
    num_turns: u32,
) -> Result<(), String> {
    let key = native_key(runtime_key, role_name, app_session_id);
    let Some(slot) = native_sessions().get(&key).map(|entry| entry.clone()) else {
        return Err("no active codex session slot for this app session".to_string());
    };
    let mut guard = slot.lock().await;
    let Some(session) = guard.as_mut() else {
        return Err("no active codex session slot for this app session".to_string());
    };
    if session.protocol != NativeProtocol::CodexAppServer {
        return Err("thread rollback is only supported for native Codex".to_string());
    }
    let Some(thread_id) = session.session_id.clone() else {
        return Err("no provider thread bound to this session".to_string());
    };
    codex_admin::rollback_codex_thread(&mut session.process, &thread_id, num_turns).await
}

pub(crate) async fn shutdown_native_sessions() {
    let keys = native_sessions()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for key in keys {
        let Some(lock) = native_sessions().get(&key).map(|entry| entry.clone()) else {
            continue;
        };
        let _ = native_cancelled().insert(key.clone());
        if let Some(pid) = native_children().get(&key).map(|entry| *entry) {
            terminate_pid(pid);
        }
        let mut slot = lock.lock().await;
        close_native_session(&mut slot, &key).await;
        native_sessions().remove(&key);
        native_active().remove(&key);
        native_cancelled().remove(&key);
    }
}

pub(crate) fn reclaim_idle_native_sessions() {
    let now = Instant::now();
    let keys = native_sessions()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for key in keys {
        if native_active().contains(&key) {
            continue;
        }
        let Some(slot) = native_sessions().get(&key).map(|entry| entry.clone()) else {
            continue;
        };
        let Ok(mut session) = slot.try_lock() else {
            continue;
        };
        let expired = session
            .as_ref()
            .is_some_and(|session| now.duration_since(session.last_used) >= NATIVE_IDLE_AFTER);
        if !expired {
            continue;
        }
        if let Some(session) = session.take() {
            native_writers().remove(&key);
            clear_active_native_turn(&key);
            if let Some(pid) = session.process.pid() {
                terminate_pid(pid);
                native_children().remove(&key);
                unregister_child_pid(pid);
            }
        }
        native_sessions().remove(&key);
        acp_log("native.idle_reclaim", json!({ "key": key }));
    }
}

struct NativeSession {
    process: NativeProcess,
    protocol: NativeProtocol,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
    session_id: Option<String>,
    initialized: bool,
    commands_loaded: bool,
    last_used: Instant,
}

impl NativeSession {
    fn matches(
        &mut self,
        protocol: NativeProtocol,
        binary: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
        resume_session_id: Option<&str>,
    ) -> bool {
        let args_match = if protocol == NativeProtocol::PiRpc {
            pi_args_without_session(&self.args) == pi_args_without_session(args)
        } else {
            self.args.as_slice() == args
        };
        let resume_match = if protocol != NativeProtocol::PiRpc {
            true
        } else {
            match resume_session_id {
                None => true,
                Some(expected) => self.session_id.as_deref() == Some(expected),
            }
        };
        let matches = self.protocol == protocol
            && self.binary == binary
            && args_match
            && self.env.as_slice() == env
            && self.cwd == cwd
            && resume_match
            && self.process.is_alive();
        if matches {
            self.last_used = Instant::now();
        }
        matches
    }
}

#[derive(Debug, Clone)]
pub(super) struct NativeCatalog {
    pub(super) options: Vec<Value>,
    pub(super) modes: Vec<String>,
    pub(super) session_id: String,
}

pub(super) struct NativeRunRequest<'a> {
    pub(super) protocol: NativeProtocol,
    pub(super) runtime_key: &'static str,
    pub(super) role_name: &'a str,
    pub(super) prompt: &'a str,
    pub(super) context: &'a [(String, String)],
    pub(super) attachments: &'a [crate::types::ImageAttachment],
    pub(super) cwd: &'a str,
    pub(super) app: &'a tauri::AppHandle,
    pub(super) auto_approve: bool,
    pub(super) role_mode: Option<&'a str>,
    pub(super) role_config_options: &'a [(String, String)],
    pub(super) binary: &'a str,
    pub(super) adapter_args: &'a [String],
    pub(super) env: &'a [(String, String)],
    pub(super) resume_session_id: Option<&'a str>,
    pub(super) mcp_servers: &'a [acp::McpServer],
    pub(super) app_session_id: &'a str,
    pub(super) turn_id: &'a str,
}

pub(super) struct NativeEventSink<'a> {
    app: &'a tauri::AppHandle,
    role: &'a str,
    runtime: &'static str,
    app_session_id: &'a str,
    turn_id: &'a str,
    sequence: u32,
    delta_batch: String,
    last_delta_flush: Instant,
}

impl<'a> NativeEventSink<'a> {
    fn new(request: &NativeRunRequest<'a>) -> Self {
        Self {
            app: request.app,
            role: request.role_name,
            runtime: request.runtime_key,
            app_session_id: request.app_session_id,
            turn_id: request.turn_id,
            sequence: 0,
            delta_batch: String::new(),
            last_delta_flush: Instant::now(),
        }
    }

    pub(super) fn push_text(&mut self, text: &str, output: &mut String) {
        output.push_str(text);
        self.delta_batch.push_str(text);
        if self.delta_batch.len() >= NATIVE_DELTA_BATCH_BYTES
            || self.last_delta_flush.elapsed() >= NATIVE_DELTA_FLUSH_AFTER
        {
            self.flush_text();
        }
    }

    pub(super) fn emit(&mut self, event: AcpEvent) {
        self.flush_text();
        emit_native_event(
            self.app,
            self.role,
            self.runtime,
            self.app_session_id,
            self.turn_id,
            &mut self.sequence,
            event,
        );
    }

    pub(super) fn flush_text(&mut self) {
        if self.delta_batch.is_empty() {
            return;
        }
        emit_text(
            self.app,
            self.role,
            self.runtime,
            self.app_session_id,
            self.turn_id,
            &self.delta_batch,
        );
        self.delta_batch.clear();
        self.last_delta_flush = Instant::now();
    }

    pub(super) fn sequence(&self) -> u32 {
        self.sequence
    }

    pub(super) fn identity(&self) -> (&'static str, &str, &str) {
        (self.runtime, self.role, self.app_session_id)
    }
}

impl Drop for NativeEventSink<'_> {
    fn drop(&mut self) {
        self.flush_text();
    }
}

pub(super) async fn execute_native_runtime(request: NativeRunRequest<'_>) -> AcpPromptResult {
    let key = native_key(
        request.runtime_key,
        request.role_name,
        request.app_session_id,
    );
    let lock = native_locks()
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    native_active().insert(key.clone());
    native_cancelled().remove(&key);
    let result = execute_native_inner(&request, &key).await;
    clear_active_native_turn(&key);
    native_active().remove(&key);
    native_cancelled().remove(&key);
    result
}

async fn execute_native_inner(request: &NativeRunRequest<'_>, key: &str) -> AcpPromptResult {
    let protocol = request.protocol;
    if native_cancelled().contains(key) {
        return native_error(
            request.runtime_key,
            request.role_name,
            request.app_session_id,
            request.turn_id,
            request.app,
            AcpErrorCode::RequestCancelled,
            "native prompt cancelled",
            json!({ "mode": "native", "runtimeKey": request.runtime_key }),
            0,
        );
    }

    let args = native_launch_args(request);
    let diagnostic = match protocol {
        NativeProtocol::CodexAppServer => "Codex app-server",
        NativeProtocol::PiRpc => "Pi RPC",
    };
    let slot = native_sessions()
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(None)))
        .clone();
    let mut slot = slot.lock().await;
    let reused = slot.as_mut().is_some_and(|session| {
        session.matches(
            protocol,
            request.binary,
            &args,
            request.env,
            request.cwd,
            request.resume_session_id,
        )
    });
    if !reused {
        close_native_session(&mut slot, key).await;
        let process = match NativeProcess::spawn(
            request.binary,
            &args,
            request.env,
            request.cwd,
            diagnostic,
        ) {
            Ok(process) => process,
            Err(error) => {
                return native_error(
                    request.runtime_key,
                    request.role_name,
                    request.app_session_id,
                    request.turn_id,
                    request.app,
                    AcpErrorCode::ProcessCrashed,
                    error,
                    json!({ "mode": "native", "transport": transport_name(protocol), "runtimeKey": request.runtime_key }),
                    0,
                )
            }
        };
        if let Some(pid) = process.pid() {
            native_children().insert(key.to_string(), pid);
            register_child_pid(pid);
        }
        native_writers().insert(key.to_string(), process.stdin_handle());
        *slot = Some(NativeSession {
            process,
            protocol,
            binary: request.binary.to_string(),
            args,
            env: request.env.to_vec(),
            cwd: request.cwd.to_string(),
            session_id: None,
            initialized: false,
            commands_loaded: false,
            last_used: Instant::now(),
        });
    }
    if native_cancelled().contains(key) {
        close_native_session(&mut slot, key).await;
        native_sessions().remove(key);
        return native_error(
            request.runtime_key,
            request.role_name,
            request.app_session_id,
            request.turn_id,
            request.app,
            AcpErrorCode::RequestCancelled,
            "native prompt cancelled",
            json!({ "mode": "native", "runtimeKey": request.runtime_key }),
            0,
        );
    }
    let mut sink = NativeEventSink::new(request);
    let resume_id = request.resume_session_id.filter(|s| !s.trim().is_empty());
    sink.emit(AcpEvent::StatusUpdate {
        text: if reused {
            if let Some(sid) = resume_id {
                format!("Reusing {diagnostic} session ({sid})...")
            } else {
                format!("Reusing {diagnostic} session...")
            }
        } else if let Some(sid) = resume_id {
            format!("Resuming {diagnostic} session ({sid})...")
        } else {
            format!("Initializing new {diagnostic} session...")
        },
    });

    let result = {
        let session = slot
            .as_mut()
            .expect("native session must exist after spawn or reuse");
        let result = match protocol {
            NativeProtocol::CodexAppServer => {
                codex::run(
                    &mut session.process,
                    request,
                    &mut sink,
                    !session.initialized,
                    key,
                )
                .await
            }
            NativeProtocol::PiRpc => {
                pi::run(
                    &mut session.process,
                    request,
                    &mut sink,
                    !session.commands_loaded,
                    key,
                )
                .await
            }
        };
        if result.is_ok() {
            match protocol {
                NativeProtocol::CodexAppServer => session.initialized = true,
                NativeProtocol::PiRpc => session.commands_loaded = true,
            }
            if let Ok((_, session_id, _)) = &result {
                if !session_id.trim().is_empty() {
                    session.session_id = Some(session_id.clone());
                }
            }
        }
        session.last_used = Instant::now();
        result
    };
    sink.flush_text();
    if result.is_err() || native_cancelled().contains(key) {
        close_native_session(&mut slot, key).await;
        native_sessions().remove(key);
    }

    match result {
        Ok((output, session_id, sequence)) => AcpPromptResult {
            ok: true,
            output,
            error_code: None,
            deltas: vec![],
            meta: json!({
                "mode": "native",
                "transport": transport_name(protocol),
                "runtimeKey": request.runtime_key,
                "sessionId": session_id,
                "eventCount": sequence,
                "mcpServerCount": request.mcp_servers.len(),
            }),
            session_handle: Some(session_id),
        },
        Err(error) => native_error(
            request.runtime_key,
            request.role_name,
            request.app_session_id,
            request.turn_id,
            request.app,
            if native_cancelled().contains(key) {
                AcpErrorCode::RequestCancelled
            } else if error.to_ascii_lowercase().contains("timed out") {
                AcpErrorCode::PromptTimeout
            } else if error.to_ascii_lowercase().contains("process exited") {
                AcpErrorCode::ProcessCrashed
            } else {
                classify_native_error(&error)
            },
            error,
            json!({ "mode": "native", "transport": transport_name(protocol), "runtimeKey": request.runtime_key }),
            sink.sequence(),
        ),
    }
}

fn native_launch_args(request: &NativeRunRequest<'_>) -> Vec<String> {
    let mut args = request.adapter_args.to_vec();
    if request.protocol == NativeProtocol::PiRpc {
        append_pi_session_arg(&mut args, request.resume_session_id);
    }
    args
}

async fn close_native_session(slot: &mut Option<NativeSession>, key: &str) {
    let Some(mut session) = slot.take() else {
        return;
    };
    let pid = session.process.pid();
    session.process.close().await;
    native_writers().remove(key);
    clear_active_native_turn(key);
    if let Some(pid) = pid {
        native_children().remove(key);
        unregister_child_pid(pid);
    }
}

pub(super) async fn refresh_native_catalog(
    protocol: NativeProtocol,
    runtime_key: &'static str,
    binary: &str,
    adapter_args: &[String],
    env: &[(String, String)],
    cwd: &str,
    role_config_options: &[(String, String)],
    resume_session_id: Option<String>,
    auto_approve: bool,
    force_refresh: bool,
) -> NativeCatalog {
    // Discovery here costs a full `codex app-server` / `pi --mode rpc` subprocess, and prewarm
    // runs on every persona switch and session resume. Reuse the process-lifetime catalog
    // unless the caller explicitly asked to re-read it from the binary — the same rule the
    // headless catalog already follows. Callers that need a provider session id (Pi resume)
    // pass `force_refresh` so they still spawn and get one.
    // Reuse only a catalog that is actually complete and still fresh — `fresh_config_options`
    // owns that judgement, so this cannot drift back into asking whether models happen to be
    // known (they are populated by any chat turn, with no option set attached).
    if !force_refresh {
        if let Some(options) = fresh_config_options(runtime_key) {
            return NativeCatalog {
                options,
                modes: list_discovered_modes(runtime_key),
                session_id: String::new(),
            };
        }
    }
    clear_runtime(runtime_key);
    let mut args = adapter_args.to_vec();
    if protocol == NativeProtocol::PiRpc {
        append_pi_launch_config(&mut args, role_config_options, resume_session_id.as_deref());
    }
    let Ok(mut process) = NativeProcess::spawn(binary, &args, env, cwd, transport_name(protocol))
    else {
        return NativeCatalog {
            options: vec![],
            modes: vec![],
            session_id: String::new(),
        };
    };
    let process_pid = process.pid();
    if let Some(pid) = process_pid {
        register_child_pid(pid);
    }
    let catalog = match protocol {
        NativeProtocol::CodexAppServer => {
            codex::refresh_catalog(&mut process, runtime_key, auto_approve).await
        }
        NativeProtocol::PiRpc => {
            // Read from the binary that will run, not from a list maintained in here.
            let thinking = super::cli_help::levels_from_help(binary, env, cwd, "--thinking")
                .await
                .unwrap_or_default();
            pi::refresh_catalog(&mut process, runtime_key, auto_approve, &thinking).await
        }
    };
    remember_runtime_config_options(runtime_key, catalog.options.clone());
    remember_runtime_modes(runtime_key, catalog.modes.clone());
    process.close().await;
    if let Some(pid) = process_pid {
        unregister_child_pid(pid);
    }
    catalog
}

fn append_pi_launch_config(
    args: &mut Vec<String>,
    options: &[(String, String)],
    session_id: Option<&str>,
) {
    if let Some(model) = find_option(options, &["model", "model_id"]) {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        args.extend(["--session".to_string(), session_id.to_string()]);
    }
}

fn append_pi_session_arg(args: &mut Vec<String>, session_id: Option<&str>) {
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        args.extend(["--session".to_string(), session_id.to_string()]);
    }
}

fn pi_args_without_session(args: &[String]) -> Vec<&str> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--session" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--session=") {
            continue;
        }
        filtered.push(arg.as_str());
    }
    filtered
}

pub(super) fn extract_pi_session_id(value: &Value) -> Option<String> {
    value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .or_else(|| value.get("sessionFile"))
        .or_else(|| value.get("session_file"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn find_option(options: &[(String, String)], names: &[&str]) -> Option<String> {
    options
        .iter()
        .find(|(key, value)| {
            !value.trim().is_empty() && names.iter().any(|name| key.eq_ignore_ascii_case(name))
        })
        .map(|(_, value)| value.clone())
}

/// A control-plane call a parameter declared as its delivery mechanism.
#[derive(Debug, PartialEq)]
pub(super) struct WiredRpc {
    pub(super) method: String,
    pub(super) field: String,
    pub(super) value: String,
}

/// The user's stored values, each resolved through the `wire` its runtime declared for it.
#[derive(Debug, Default, PartialEq)]
pub(super) struct WiredValues {
    pub(super) turn_params: Vec<(String, String)>,
    pub(super) rpc_calls: Vec<WiredRpc>,
    pub(super) model_suffixes: Vec<String>,
    pub(super) cli_settings: Vec<Value>,
}

impl WiredValues {
    pub(super) fn turn_param(&self, name: &str) -> Option<&str> {
        self.turn_params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn rpc(&self, method: &str) -> Option<&WiredRpc> {
        self.rpc_calls.iter().find(|call| call.method == method)
    }
}

fn stored_value<'a>(config: &'a [(String, String)], id: &str) -> Option<&'a str> {
    config
        .iter()
        .find(|(key, value)| key.eq_ignore_ascii_case(id) && !value.trim().is_empty())
        .map(|(_, value)| value.trim())
}

fn toggle_is_on(value: &str) -> bool {
    matches!(value.trim(), "true" | "1" | "on")
}

/// Resolve stored values against the runtime's own declaration of its parameters.
///
/// The send path used to guess which stored key meant "effort" by trying a list of plausible
/// names (`["effort", "reasoning_effort", "reasoningEffort"]`). That guess existed because the
/// id a value is stored under and the field the CLI expects genuinely differ — Codex stores
/// `reasoning_effort` but `turn/start` takes `effort` — and every new runtime widened the
/// guess. Now the runtime states the mapping in `wire` and this just follows it.
///
/// An option the catalog does not declare is ignored: the catalog is the list of parameters
/// the runtime accepts, so a stored key outside it has nowhere legitimate to go.
pub(super) fn resolve_wired_values(catalog: &[Value], config: &[(String, String)]) -> WiredValues {
    let mut out = WiredValues::default();
    for option in catalog {
        let Some(id) = option.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(wire) = option.get("wire") else {
            continue;
        };
        let Some(stored) = stored_value(config, id) else {
            continue;
        };
        let is_toggle = option.get("kind").and_then(Value::as_str) == Some("toggle");
        if is_toggle && !toggle_is_on(stored) {
            continue;
        }
        match wire.get("kind").and_then(Value::as_str) {
            Some("turn_param") => {
                let Some(name) = wire.get("name").and_then(Value::as_str) else {
                    continue;
                };
                // A toggle carries the value to send when on; a select sends what was chosen.
                let value = if is_toggle {
                    match wire.get("on_value").and_then(Value::as_str) {
                        Some(on_value) => on_value.to_string(),
                        None => continue,
                    }
                } else {
                    stored.to_string()
                };
                out.turn_params.push((name.to_string(), value));
            }
            Some("rpc") => {
                let (Some(method), Some(field)) = (
                    wire.get("method").and_then(Value::as_str),
                    wire.get("field").and_then(Value::as_str),
                ) else {
                    continue;
                };
                out.rpc_calls.push(WiredRpc {
                    method: method.to_string(),
                    field: field.to_string(),
                    value: stored.to_string(),
                });
            }
            Some("model_suffix") => {
                if let Some(suffix) = wire.get("suffix").and_then(Value::as_str) {
                    out.model_suffixes.push(suffix.to_string());
                }
            }
            Some("cli_settings") => {
                if let Some(settings) = wire.get("json") {
                    out.cli_settings.push(settings.clone());
                }
            }
            _ => {}
        }
    }
    out
}

pub(super) fn compose_prompt(prompt: &str, context: &[(String, String)]) -> String {
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

pub(super) fn first_text(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value.get(*name).and_then(|value| {
            value.as_str().map(ToString::to_string).or_else(|| {
                value
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
        })
    })
}

pub(super) fn emit_text(
    app: &tauri::AppHandle,
    role: &str,
    runtime: &'static str,
    app_session_id: &str,
    turn_id: &str,
    text: &str,
) {
    let _ = app.emit(
        "acp/delta",
        AcpDeltaPayload {
            schema_version: 1,
            role,
            runtime_kind: runtime,
            app_session_id,
            turn_id,
            delta: text,
        },
    );
}

pub(super) fn emit_native_event(
    app: &tauri::AppHandle,
    role: &str,
    runtime: &'static str,
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
            role,
            runtime_kind: runtime,
            app_session_id,
            turn_id,
            event: &event,
            seq: *sequence,
        },
    );
}

fn native_error(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
    turn_id: &str,
    app: &tauri::AppHandle,
    code: AcpErrorCode,
    raw: impl Into<String>,
    meta: Value,
    sequence: u32,
) -> AcpPromptResult {
    let raw = raw.into();
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
            seq: sequence,
        },
    );
    acp_log(
        "native.execute.error",
        json!({ "runtime": runtime_key, "role": role_name, "code": code.as_str(), "error": raw }),
    );
    AcpPromptResult {
        ok: false,
        output: crate::acp::adapter::friendly_error_message(runtime_key, &raw),
        error_code: Some(code.as_str().to_string()),
        deltas: vec![],
        meta,
        session_handle: None,
    }
}

fn classify_native_error(error: &str) -> AcpErrorCode {
    let lower = error.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("login")
        || lower.contains("credential")
        || lower.contains("api key")
    {
        AcpErrorCode::AuthRequired
    } else if lower.contains("invalid") || lower.contains("unsupported") {
        AcpErrorCode::InvalidParams
    } else {
        AcpErrorCode::AgentError
    }
}

#[cfg(test)]
mod wire_tests {
    use super::super::option_spec::{OptionSpec, OptionValue, Wire};
    use super::*;

    fn catalog() -> Vec<Value> {
        vec![
            OptionSpec::select(
                // Codex's real mismatch: stored under `reasoning_effort`, delivered as `effort`.
                "reasoning_effort",
                "Effort",
                Wire::TurnParam {
                    name: "effort".to_string(),
                    on_value: None,
                },
                vec![OptionValue::new("high", "high")],
            )
            .to_value(),
            OptionSpec::toggle(
                "fast",
                "Fast mode",
                Wire::TurnParam {
                    name: "serviceTier".to_string(),
                    on_value: Some("priority".to_string()),
                },
            )
            .to_value(),
            OptionSpec::select(
                "thinking_level",
                "Thinking",
                Wire::Rpc {
                    method: "set_thinking_level".to_string(),
                    field: "level".to_string(),
                },
                vec![OptionValue::new("xhigh", "xhigh")],
            )
            .to_value(),
        ]
    }

    fn config(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_stored_value_is_delivered_under_the_name_its_runtime_declared() {
        let wired = resolve_wired_values(&catalog(), &config(&[("reasoning_effort", "high")]));
        // Stored as `reasoning_effort`, sent as `effort` — no name guessing involved.
        assert_eq!(wired.turn_param("effort"), Some("high"));
        assert_eq!(wired.turn_param("reasoning_effort"), None);
    }

    #[test]
    fn a_toggle_sends_the_declared_tier_only_when_it_is_on() {
        let on = resolve_wired_values(&catalog(), &config(&[("fast", "true")]));
        assert_eq!(on.turn_param("serviceTier"), Some("priority"));

        for off in ["false", "", "0"] {
            let wired = resolve_wired_values(&catalog(), &config(&[("fast", off)]));
            assert_eq!(
                wired.turn_param("serviceTier"),
                None,
                "an off toggle must send nothing (value {off:?})"
            );
        }
    }

    #[test]
    fn a_parameter_delivered_by_rpc_is_not_mistaken_for_a_turn_parameter() {
        let wired = resolve_wired_values(&catalog(), &config(&[("thinking_level", "xhigh")]));
        assert!(wired.turn_params.is_empty());
        let call = wired.rpc("set_thinking_level").expect("declared rpc");
        assert_eq!(call.field, "level");
        assert_eq!(call.value, "xhigh");
    }

    #[test]
    fn a_key_the_runtime_never_declared_is_not_sent() {
        // The catalog is the set of parameters the runtime accepts, so a leftover key from
        // another engine has nowhere legitimate to go.
        let wired = resolve_wired_values(&catalog(), &config(&[("one_million", "true")]));
        assert_eq!(wired, WiredValues::default());
    }

    #[test]
    fn an_empty_catalog_yields_nothing_so_callers_fall_back() {
        // Discovery may not have run yet in this process; the caller then uses its own
        // fallback rather than dropping the user's settings.
        let wired = resolve_wired_values(&[], &config(&[("reasoning_effort", "high")]));
        assert_eq!(wired, WiredValues::default());
    }
}
