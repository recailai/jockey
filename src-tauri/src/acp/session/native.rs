use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::Mutex;

use super::super::adapter::{acp_log, NativeProtocol};
use super::super::error::AcpErrorCode;
use super::super::protocol as acp;
use super::super::runtime_state::{
    clear_runtime, remember_runtime_config_options, remember_runtime_modes,
};
use super::super::worker::{register_child_pid, unregister_child_pid, AcpEvent, AcpPromptResult};
use super::execute::{AcpDeltaPayload, AcpStreamPayload};

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

fn native_key(runtime_key: &str, role_name: &str, app_session_id: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

pub(super) fn cancel_native(runtime_key: &str, role_name: &str, app_session_id: &str) -> bool {
    let key = native_key(runtime_key, role_name, app_session_id);
    if !native_active().contains(&key) {
        return false;
    }
    native_cancelled().insert(key.clone());
    if let Some(pid) = native_children().get(&key).map(|entry| *entry) {
        terminate_pid(pid);
        acp_log(
            "native.cancel",
            json!({ "runtime": runtime_key, "role": role_name, "pid": pid }),
        );
    }
    true
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
}

pub(super) struct NativeEventSink<'a> {
    app: &'a tauri::AppHandle,
    role: &'a str,
    runtime: &'static str,
    app_session_id: &'a str,
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
            &self.delta_batch,
        );
        self.delta_batch.clear();
        self.last_delta_flush = Instant::now();
    }

    pub(super) fn sequence(&self) -> u32 {
        self.sequence
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
            request.app,
            AcpErrorCode::RequestCancelled,
            "native prompt cancelled",
            json!({ "mode": "native", "runtimeKey": request.runtime_key }),
            0,
        );
    }
    let mut sink = NativeEventSink::new(request);
    sink.emit(AcpEvent::StatusUpdate {
        text: if reused {
            format!("Reusing {diagnostic} session...")
        } else {
            format!("Connecting to {diagnostic}...")
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
                )
                .await
            }
            NativeProtocol::PiRpc => {
                pi::run(
                    &mut session.process,
                    request,
                    &mut sink,
                    !session.commands_loaded,
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
        },
        Err(error) => native_error(
            request.runtime_key,
            request.role_name,
            request.app_session_id,
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
) -> NativeCatalog {
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
        NativeProtocol::PiRpc => pi::refresh_catalog(&mut process, runtime_key, auto_approve).await,
    };
    remember_runtime_config_options(runtime_key, catalog.options.clone());
    remember_runtime_modes(runtime_key, catalog.modes.clone());
    process.close().await;
    if let Some(pid) = process_pid {
        unregister_child_pid(pid);
    }
    catalog
}

pub(super) fn native_model_options(
    models: &[String],
    secondary_id: &str,
    secondary: &[&str],
) -> Vec<Value> {
    let model_values = models
        .iter()
        .map(|model| json!({ "value": model, "name": model }))
        .collect::<Vec<_>>();
    let mut options = vec![json!({
        "id": "model",
        "name": "Model",
        "description": "Discovered from the native runtime",
        "category": "model",
        "type": "select",
        "currentValue": "",
        "options": model_values,
    })];
    if !secondary.is_empty() {
        options.push(json!({
            "id": secondary_id,
            "name": secondary_id,
            "category": "effort",
            "type": "select",
            "currentValue": "",
            "options": secondary.iter().map(|value| json!({ "value": value, "name": value })).collect::<Vec<_>>(),
        }));
    }
    options
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
        .get("sessionFile")
        .or_else(|| value.get("session_file"))
        .or_else(|| value.get("sessionId"))
        .or_else(|| value.get("session_id"))
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
    text: &str,
) {
    let _ = app.emit(
        "acp/delta",
        AcpDeltaPayload {
            role,
            runtime_kind: runtime,
            app_session_id,
            delta: text,
        },
    );
}

pub(super) fn emit_native_event(
    app: &tauri::AppHandle,
    role: &str,
    runtime: &'static str,
    app_session_id: &str,
    sequence: &mut u32,
    event: AcpEvent,
) {
    *sequence = sequence.saturating_add(1);
    let _ = app.emit(
        "acp/stream",
        AcpStreamPayload {
            role,
            runtime_kind: runtime,
            app_session_id,
            event: &event,
            seq: *sequence,
        },
    );
}

fn native_error(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
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
            role: role_name,
            runtime_kind: runtime_key,
            app_session_id,
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
