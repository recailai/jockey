use agent_client_protocol::{self as acp, Agent as _};
use dashmap::{DashMap, DashSet};
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::adapter::{acp_log, resolve_cwd};
use super::client::shutdown_terminals;
use super::session::cold_start;
pub use crate::runtime_kind::RuntimeKind;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPromptResult {
    pub output: String,
    pub deltas: Vec<String>,
    pub meta: Value,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AcpEvent {
    TextDelta {
        text: String,
    },
    ThoughtDelta {
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        tool_kind: String,
        status: String,
        content: Option<Vec<Value>>,
        locations: Option<Vec<Value>>,
        raw_input: Option<Value>,
        raw_output: Option<Value>,
    },
    ToolCallUpdate {
        tool_call_id: String,
        tool_kind: Option<String>,
        status: Option<String>,
        title: Option<String>,
        content: Option<Vec<Value>>,
        locations: Option<Vec<Value>>,
        raw_input: Option<Value>,
        raw_output: Option<Value>,
    },
    Plan {
        entries: Vec<Value>,
    },
    PermissionRequest {
        request_id: String,
        title: String,
        description: Option<String>,
        options: Vec<Value>,
    },
    ModeUpdate {
        mode_id: String,
    },
    ConfigUpdate {
        options: Vec<Value>,
    },
    SessionInfo {
        title: Option<String>,
    },
    StatusUpdate {
        text: String,
    },
    AvailableCommands {
        commands: Vec<Value>,
    },
    AvailableModes {
        modes: Vec<Value>,
        current: Option<String>,
    },
}

static PERMISSION_REQUESTS: OnceLock<
    DashMap<String, oneshot::Sender<acp::RequestPermissionOutcome>>,
> = OnceLock::new();
pub(super) fn permission_requests(
) -> &'static DashMap<String, oneshot::Sender<acp::RequestPermissionOutcome>> {
    PERMISSION_REQUESTS.get_or_init(DashMap::new)
}

pub fn respond_to_permission(request_id: &str, outcome: acp::RequestPermissionOutcome) {
    if let Some((_, tx)) = permission_requests().remove(request_id) {
        let _ = tx.send(outcome);
    }
}

pub(super) type DeltaSlot = Arc<Mutex<Option<mpsc::UnboundedSender<AcpEvent>>>>;

pub(super) enum WorkerMsg {
    Execute {
        runtime_key: &'static str,
        binary: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        role_name: String,
        app_session_id: String,
        prompt: String,
        context: Vec<(String, String)>,
        cwd: String,
        delta_tx: mpsc::UnboundedSender<AcpEvent>,
        result_tx: oneshot::Sender<Result<(String, String), String>>,
        auto_approve: bool,
        mcp_servers: Vec<acp::McpServer>,
        role_mode: Option<String>,
        role_config_options: Vec<(String, String)>,
        resume_session_id: Option<String>,
    },
    Prewarm {
        runtime_key: &'static str,
        binary: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        role_name: String,
        app_session_id: String,
        cwd: String,
        auto_approve: bool,
        mcp_servers: Vec<acp::McpServer>,
        role_mode: Option<String>,
        role_config_options: Vec<(String, String)>,
        result_tx: Option<oneshot::Sender<(Vec<Value>, String)>>,
        resume_session_id: Option<String>,
    },
    Cancel {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
    },
    SetMode {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
        mode_id: String,
        result_tx: oneshot::Sender<Result<(), String>>,
    },
    SetConfigOption {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
        config_id: String,
        value: String,
        result_tx: oneshot::Sender<Result<(), String>>,
    },
    Shutdown {
        done_tx: oneshot::Sender<()>,
    },
}

static WORKER_TX: OnceLock<mpsc::UnboundedSender<WorkerMsg>> = OnceLock::new();
static RUNTIME_MODELS: OnceLock<DashMap<String, Vec<String>>> = OnceLock::new();
static RUNTIME_MODES: OnceLock<DashMap<String, Vec<String>>> = OnceLock::new();
static RUNTIME_CONFIG_OPTIONS: OnceLock<DashMap<String, Vec<Value>>> = OnceLock::new();
static RUNTIME_AVAILABLE_COMMANDS: OnceLock<DashMap<String, Vec<Value>>> = OnceLock::new();

pub(super) fn worker_tx() -> &'static mpsc::UnboundedSender<WorkerMsg> {
    WORKER_TX.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel::<WorkerMsg>();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("acp worker runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, run_worker(rx));
        });
        tx
    })
}

pub async fn shutdown() {
    let Some(tx) = WORKER_TX.get() else {
        shutdown_terminals().await;
        return;
    };
    let (done_tx, done_rx) = oneshot::channel();
    if tx.send(WorkerMsg::Shutdown { done_tx }).is_err() {
        shutdown_terminals().await;
        return;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), done_rx).await;
}

fn runtime_models() -> &'static DashMap<String, Vec<String>> {
    RUNTIME_MODELS.get_or_init(DashMap::new)
}

fn runtime_modes() -> &'static DashMap<String, Vec<String>> {
    RUNTIME_MODES.get_or_init(DashMap::new)
}

fn runtime_config_options() -> &'static DashMap<String, Vec<Value>> {
    RUNTIME_CONFIG_OPTIONS.get_or_init(DashMap::new)
}

pub(super) fn remember_runtime_models(runtime_key: &str, mut models: Vec<String>) {
    if models.is_empty() {
        return;
    }
    models.sort_unstable();
    models.dedup();
    runtime_models().insert(runtime_key.to_string(), models);
}

pub(super) fn remember_runtime_modes(runtime_key: &str, mut modes: Vec<String>) {
    if modes.is_empty() {
        return;
    }
    modes.sort_unstable();
    modes.dedup();
    runtime_modes().insert(runtime_key.to_string(), modes);
}

pub fn list_discovered_models(runtime_key: &str) -> Vec<String> {
    runtime_models()
        .get(runtime_key)
        .map(|v| v.clone())
        .unwrap_or_default()
}

pub fn list_discovered_modes(runtime_key: &str) -> Vec<String> {
    runtime_modes()
        .get(runtime_key)
        .map(|v| v.clone())
        .unwrap_or_default()
}

pub(super) fn remember_runtime_config_options(runtime_key: &str, options: Vec<Value>) {
    if options.is_empty() {
        return;
    }
    runtime_config_options().insert(runtime_key.to_string(), options);
}

pub fn list_discovered_config_options(runtime_key: &str) -> Vec<Value> {
    runtime_config_options()
        .get(runtime_key)
        .map(|v| v.clone())
        .unwrap_or_default()
}

fn runtime_available_commands() -> &'static DashMap<String, Vec<Value>> {
    RUNTIME_AVAILABLE_COMMANDS.get_or_init(DashMap::new)
}

pub(super) fn remember_runtime_available_commands(
    runtime_key: &str,
    role_name: &str,
    commands: Vec<Value>,
) {
    runtime_available_commands().insert(commands_key(runtime_key, role_name), commands);
}

pub fn list_available_commands(runtime_key: &str, role_name: &str) -> Vec<Value> {
    runtime_available_commands()
        .get(&commands_key(runtime_key, role_name))
        .map(|v| v.clone())
        .unwrap_or_default()
}

fn commands_key(runtime_key: &str, role_name: &str) -> String {
    format!("{runtime_key}:{role_name}")
}

fn pool_key(app_session_id: &str, runtime_key: &str, role_name: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

pub(super) struct SlotHandle {
    pub(super) conn: tokio::sync::Mutex<Option<LiveConnection>>,
}

/// A lightweight handle for sending ACP cancel without holding the conn mutex.
/// Stored in a thread-local map keyed by pool_key, only accessed on the worker
/// thread's LocalSet — so `Rc` is fine.
struct CancelHandle {
    conn: Rc<acp::ClientSideConnection>,
    session_id: acp::SessionId,
}

thread_local! {
    static CANCEL_HANDLES: RefCell<std::collections::HashMap<String, CancelHandle>> =
        RefCell::new(std::collections::HashMap::new());
}

static SLOT_MAP: OnceLock<Arc<DashMap<String, Arc<SlotHandle>>>> = OnceLock::new();

fn slot_map() -> &'static Arc<DashMap<String, Arc<SlotHandle>>> {
    SLOT_MAP.get_or_init(|| Arc::new(DashMap::new()))
}

static CHILD_PIDS: OnceLock<DashSet<u32>> = OnceLock::new();
fn child_pids() -> &'static DashSet<u32> {
    CHILD_PIDS.get_or_init(DashSet::new)
}

pub(super) fn register_child_pid(pid: u32) {
    child_pids().insert(pid);
}

pub(super) fn unregister_child_pid(pid: u32) {
    child_pids().remove(&pid);
}

fn get_slot_handle(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Arc<SlotHandle> {
    let key = pool_key(app_session_id, runtime_key, role_name);
    slot_map()
        .entry(key)
        .or_insert_with(|| {
            Arc::new(SlotHandle {
                conn: tokio::sync::Mutex::new(None),
            })
        })
        .clone()
}

pub(super) struct LiveConnection {
    pub(super) conn: Rc<acp::ClientSideConnection>,
    pub(super) session_id: acp::SessionId,
    pub(super) cwd: String,
    pub(super) delta_slot: DeltaSlot,
    #[allow(dead_code)]
    pub(super) available_modes: Vec<Value>,
    #[allow(dead_code)]
    pub(super) current_mode: Option<String>,
    pub(super) child_pid: Option<u32>,
    pub(super) _child: tokio::process::Child,
    pub(super) _io_task: tokio::task::JoinHandle<()>,
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        if let Some(pid) = self.child_pid {
            unregister_child_pid(pid);
        }
    }
}

// SAFETY: LiveConnection is only ever created and accessed on the single-threaded
// worker LocalSet (see `run_worker`).  The DashMap / tokio::sync::Mutex wrappers
// require Send+Sync at the type level, but no actual cross-thread access occurs.
// The Rc<ClientSideConnection> is the only non-Send field; it is safe because:
// 1. cold_start() runs on the worker LocalSet and produces the Rc.
// 2. handle_execute/handle_prewarm run on the same LocalSet.
// 3. The DashMap is only mutated from run_worker (also on that thread).
unsafe impl Send for LiveConnection {}
unsafe impl Sync for LiveConnection {}

async fn shutdown_worker_state() {
    let cancel_handles = CANCEL_HANDLES.with(|m| {
        let mut map = m.borrow_mut();
        let items = map
            .iter()
            .map(|(k, h)| (k.clone(), h.conn.clone(), h.session_id.clone()))
            .collect::<Vec<_>>();
        map.clear();
        items
    });
    for (_key, conn, session_id) in cancel_handles {
        let _ = conn.cancel(acp::CancelNotification::new(session_id)).await;
    }

    let slots: Vec<(String, Arc<SlotHandle>)> = slot_map()
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect();
    let mut dropped = 0usize;
    let mut timed_out = 0usize;
    for (_key, slot) in &slots {
        match tokio::time::timeout(std::time::Duration::from_millis(1200), slot.conn.lock()).await {
            Ok(mut guard) => {
                *guard = None;
                dropped += 1;
            }
            Err(_) => {
                timed_out += 1;
            }
        }
    }
    slot_map().clear();
    runtime_models().clear();
    runtime_modes().clear();
    runtime_config_options().clear();
    runtime_available_commands().clear();
    let request_ids: Vec<String> = permission_requests()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    for request_id in request_ids {
        if let Some((_, tx)) = permission_requests().remove(&request_id) {
            let _ = tx.send(acp::RequestPermissionOutcome::Cancelled);
        }
    }
    shutdown_terminals().await;

    let remaining_pids: Vec<u32> = child_pids().iter().map(|r| *r).collect();
    for pid in &remaining_pids {
        unsafe {
            libc::kill(-(*pid as i32), libc::SIGKILL);
        }
        child_pids().remove(pid);
    }

    acp_log(
        "shutdown.complete",
        json!({ "slots": slots.len(), "dropped": dropped, "timedOut": timed_out, "forceKilled": remaining_pids.len() }),
    );
}

async fn run_worker(mut rx: mpsc::UnboundedReceiver<WorkerMsg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMsg::Execute {
                runtime_key,
                binary,
                args,
                env,
                role_name,
                app_session_id,
                prompt,
                context,
                cwd,
                delta_tx,
                result_tx,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
                resume_session_id,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name, &app_session_id);
                tokio::task::spawn_local(async move {
                    handle_execute(
                        slot,
                        runtime_key,
                        binary,
                        args,
                        env,
                        role_name,
                        app_session_id,
                        prompt,
                        context,
                        cwd,
                        delta_tx,
                        result_tx,
                        auto_approve,
                        mcp_servers,
                        role_mode,
                        role_config_options,
                        resume_session_id,
                    )
                    .await;
                });
            }
            WorkerMsg::Prewarm {
                runtime_key,
                binary,
                args,
                env,
                role_name,
                app_session_id,
                cwd,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
                result_tx,
                resume_session_id,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name, &app_session_id);
                tokio::task::spawn_local(async move {
                    handle_prewarm(
                        slot,
                        runtime_key,
                        binary,
                        args,
                        env,
                        role_name,
                        cwd,
                        auto_approve,
                        mcp_servers,
                        role_mode,
                        role_config_options,
                        result_tx,
                        resume_session_id,
                    )
                    .await;
                });
            }
            WorkerMsg::Cancel {
                runtime_key,
                role_name,
                app_session_id,
            } => {
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                // Read the cancel handle from thread-local WITHOUT locking the
                // conn mutex.  Safe because we're on the worker's LocalSet.
                let handle = CANCEL_HANDLES.with(|m| {
                    m.borrow()
                        .get(&key)
                        .map(|h| (h.conn.clone(), h.session_id.clone()))
                });
                if let Some((conn, session_id)) = handle {
                    acp_log(
                        "cancel.sending",
                        json!({ "runtime": runtime_key, "role": role_name }),
                    );
                    // cancel() is a JSON-RPC notification — fire-and-forget.
                    // spawn_local so run_worker loop continues immediately.
                    tokio::task::spawn_local(async move {
                        let _ = conn.cancel(acp::CancelNotification::new(session_id)).await;
                    });
                } else {
                    acp_log(
                        "cancel.no_active_session",
                        json!({ "runtime": runtime_key, "role": role_name }),
                    );
                }
            }
            WorkerMsg::SetMode {
                runtime_key,
                role_name,
                app_session_id,
                mode_id,
                result_tx,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name, &app_session_id);
                tokio::task::spawn_local(async move {
                    let guard = slot.conn.lock().await;
                    let result = if let Some(live) = guard.as_ref() {
                        live.conn
                            .set_session_mode(acp::SetSessionModeRequest::new(
                                live.session_id.clone(),
                                acp::SessionModeId::from(mode_id),
                            ))
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    } else {
                        Err("no active session".to_string())
                    };
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::SetConfigOption {
                runtime_key,
                role_name,
                app_session_id,
                config_id,
                value,
                result_tx,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name, &app_session_id);
                tokio::task::spawn_local(async move {
                    let guard = slot.conn.lock().await;
                    let result = if let Some(live) = guard.as_ref() {
                        live.conn
                            .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                                live.session_id.clone(),
                                acp::SessionConfigId::from(config_id),
                                acp::SessionConfigValueId::from(value),
                            ))
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    } else {
                        Err("no active session".to_string())
                    };
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::Shutdown { done_tx } => {
                acp_log("shutdown.start", json!({}));
                shutdown_worker_state().await;
                let _ = done_tx.send(());
                break;
            }
        }
    }
}

async fn evict_if_cwd_changed(
    guard: &mut Option<LiveConnection>,
    resolved: &str,
    runtime_key: &str,
    role_name: &str,
) {
    if guard.as_ref().map(|c| c.cwd != resolved).unwrap_or(false) {
        acp_log(
            "pool.evict",
            json!({ "runtime": runtime_key, "role": role_name, "reason": "cwd_change" }),
        );
        *guard = None;
    }
}

async fn ensure_connection(
    guard: &mut Option<LiveConnection>,
    runtime_key: &'static str,
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    resolved: &str,
    role_name: &str,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    resume_session_id: Option<String>,
) -> Result<bool, String> {
    if guard.is_some() {
        acp_log(
            "pool.reuse",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
        return Ok(false);
    }
    let conn = cold_start(
        runtime_key,
        binary,
        args,
        env,
        resolved,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await?;
    acp_log(
        "pool.cold_start",
        json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
    );
    *guard = Some(conn);
    Ok(true)
}

async fn apply_cold_start_config(
    conn: &LiveConnection,
    session_id: &acp::SessionId,
    delta_tx: &mpsc::UnboundedSender<AcpEvent>,
    role_mode: &Option<String>,
    role_config_options: &[(String, String)],
) {
    if !conn.available_modes.is_empty() {
        let _ = delta_tx.send(AcpEvent::AvailableModes {
            modes: conn.available_modes.clone(),
            current: conn.current_mode.clone(),
        });
    }

    if let Some(mode) = role_mode {
        let _ = conn
            .conn
            .set_session_mode(acp::SetSessionModeRequest::new(
                session_id.clone(),
                acp::SessionModeId::from(mode.clone()),
            ))
            .await;
    }
    for (key, value) in role_config_options {
        let _ = conn
            .conn
            .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                session_id.clone(),
                acp::SessionConfigId::from(key.clone()),
                acp::SessionConfigValueId::from(value.clone()),
            ))
            .await;
    }
}

fn build_prompt_blocks(prompt: String, context: &[(String, String)]) -> Vec<acp::ContentBlock> {
    let mut blocks: Vec<acp::ContentBlock> =
        vec![acp::ContentBlock::Text(acp::TextContent::new(prompt))];
    if !context.is_empty() {
        let ctx = context
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(format!(
            "[context]\n{ctx}"
        ))));
    }
    blocks
}

async fn handle_execute(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    app_session_id: String,
    prompt: String,
    context: Vec<(String, String)>,
    cwd: String,
    delta_tx: mpsc::UnboundedSender<AcpEvent>,
    result_tx: oneshot::Sender<Result<(String, String), String>>,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    resume_session_id: Option<String>,
) {
    let mut guard = slot.conn.lock().await;
    let resolved = resolve_cwd(&cwd);

    evict_if_cwd_changed(&mut guard, &resolved, runtime_key, &role_name).await;

    if guard.is_none() {
        let _ = delta_tx.send(AcpEvent::StatusUpdate {
            text: "Connecting to agent runtime...".to_string(),
        });
    }

    let is_cold = match ensure_connection(
        &mut guard,
        runtime_key,
        &binary,
        &args,
        &env,
        &resolved,
        &role_name,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await
    {
        Ok(cold) => cold,
        Err(e) => {
            let _ = result_tx.send(Err(e));
            return;
        }
    };

    let conn = guard.as_mut().unwrap();
    let session_id = conn.session_id.clone();

    if let Ok(mut slot) = conn.delta_slot.lock() {
        *slot = Some(delta_tx.clone());
    }

    if is_cold {
        apply_cold_start_config(
            conn,
            &session_id,
            &delta_tx,
            &role_mode,
            &role_config_options,
        )
        .await;
    }

    let blocks = build_prompt_blocks(prompt, &context);

    // Install cancel handle so the Cancel message handler can send ACP cancel
    // without waiting for the conn mutex (which we hold right now).
    let cancel_key = pool_key(&app_session_id, runtime_key, &role_name);
    CANCEL_HANDLES.with(|m| {
        m.borrow_mut().insert(
            cancel_key.clone(),
            CancelHandle {
                conn: conn.conn.clone(),
                session_id: session_id.clone(),
            },
        );
    });

    let prompt_started = Instant::now();
    let prompt_result = conn
        .conn
        .prompt(acp::PromptRequest::new(session_id.clone(), blocks))
        .await;

    // Clear cancel handle — prompt is done.
    CANCEL_HANDLES.with(|m| {
        m.borrow_mut().remove(&cancel_key);
    });

    if let Ok(mut slot) = conn.delta_slot.lock() {
        *slot = None;
    }

    match prompt_result {
        Ok(resp) => {
            acp_log(
                "stage.ok",
                json!({
                    "runtime": runtime_key,
                    "stage": "session/prompt",
                    "latencyMs": prompt_started.elapsed().as_millis(),
                    "stopReason": format!("{:?}", resp.stop_reason)
                }),
            );
            let _ = result_tx.send(Ok((String::new(), session_id.to_string())));
        }
        Err(e) => {
            acp_log(
                "pool.invalidate",
                json!({ "runtime": runtime_key, "error": e.to_string() }),
            );
            *guard = None;
            let _ = result_tx.send(Err(e.to_string()));
        }
    }
}

async fn handle_prewarm(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    cwd: String,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    result_tx: Option<oneshot::Sender<(Vec<Value>, String)>>,
    resume_session_id: Option<String>,
) {
    let mut guard = slot.conn.lock().await;
    if guard.is_some() {
        if let Some(tx) = result_tx {
            let session_id = guard
                .as_ref()
                .map(|c| c.session_id.to_string())
                .unwrap_or_default();
            let _ = tx.send((list_discovered_config_options(runtime_key), session_id));
        }
        return;
    }
    let resolved = resolve_cwd(&cwd);
    acp_log(
        "prewarm.start",
        json!({ "runtime": runtime_key, "role": role_name }),
    );
    match cold_start(
        runtime_key,
        &binary,
        &args,
        &env,
        &resolved,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await
    {
        Ok(conn) => {
            acp_log(
                "prewarm.ok",
                json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
            );
            let session_id = conn.session_id.clone();
            let session_id_str = session_id.to_string();
            let dummy_tx = {
                let (tx, _rx) = mpsc::unbounded_channel::<AcpEvent>();
                tx
            };
            apply_cold_start_config(
                &conn,
                &session_id,
                &dummy_tx,
                &role_mode,
                &role_config_options,
            )
            .await;
            // Install a temporary drain channel so notifications sent after session/new
            // (e.g. AvailableCommandsUpdate via setTimeout) are captured rather than dropped.
            let (drain_tx, mut drain_rx) = mpsc::unbounded_channel::<super::worker::AcpEvent>();
            {
                if let Ok(mut slot_guard) = conn.delta_slot.lock() {
                    *slot_guard = Some(drain_tx);
                }
            }
            *guard = Some(conn);
            // Drain for up to 300 ms to collect any immediate notifications.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(300);
            loop {
                match tokio::time::timeout_at(deadline, drain_rx.recv()).await {
                    Ok(Some(AcpEvent::AvailableCommands { commands })) => {
                        acp_log(
                            "commands.discovered",
                            json!({
                                "runtime": runtime_key,
                                "role": role_name,
                                "count": commands.len(),
                                "names": commands.iter().filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()
                            }),
                        );
                        remember_runtime_available_commands(runtime_key, &role_name, commands);
                    }
                    Ok(Some(_)) => {}
                    _ => break,
                }
            }
            // Drop drain_rx first so the channel is closed, then clear the slot.
            drop(drain_rx);
            if let Some(live) = guard.as_ref() {
                if let Ok(mut slot_guard) = live.delta_slot.lock() {
                    *slot_guard = None;
                }
            }
            if let Some(tx) = result_tx {
                let _ = tx.send((list_discovered_config_options(runtime_key), session_id_str));
            }
        }
        Err(e) => {
            acp_log(
                "prewarm.error",
                json!({ "runtime": runtime_key, "role": role_name, "error": e }),
            );
            if let Some(tx) = result_tx {
                let _ = tx.send((vec![], String::new()));
            }
        }
    }
}
