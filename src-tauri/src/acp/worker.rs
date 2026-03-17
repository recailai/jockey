use agent_client_protocol::{self as acp, Agent as _};
use dashmap::DashMap;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::adapter::{acp_log, resolve_cwd};
use super::session::cold_start;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    Mock,
    ClaudeCode,
    GeminiCli,
    CodexCli,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPromptResult {
    pub output: String,
    pub deltas: Vec<String>,
    pub meta: Value,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
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
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: Option<String>,
        title: Option<String>,
        content: Option<Vec<Value>>,
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
        prompt: String,
        context: Vec<(String, String)>,
        cwd: String,
        delta_tx: mpsc::UnboundedSender<AcpEvent>,
        result_tx: oneshot::Sender<Result<(String, String), String>>,
        auto_approve: bool,
        mcp_servers: Vec<acp::McpServer>,
        role_mode: Option<String>,
        role_config_options: Vec<(String, String)>,
    },
    Prewarm {
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
        result_tx: Option<oneshot::Sender<Vec<Value>>>,
    },
    Cancel {
        runtime_key: &'static str,
        role_name: String,
    },
    SetMode {
        runtime_key: &'static str,
        role_name: String,
        mode_id: String,
        result_tx: oneshot::Sender<Result<(), String>>,
    },
    SetConfigOption {
        runtime_key: &'static str,
        role_name: String,
        config_id: String,
        value: String,
        result_tx: oneshot::Sender<Result<(), String>>,
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

pub(super) fn remember_runtime_available_commands(runtime_key: &str, commands: Vec<Value>) {
    runtime_available_commands().insert(runtime_key.to_string(), commands);
}

pub fn list_available_commands(runtime_key: &str) -> Vec<Value> {
    runtime_available_commands()
        .get(runtime_key)
        .map(|v| v.clone())
        .unwrap_or_default()
}

fn pool_key(runtime_key: &str, role_name: &str) -> String {
    format!("{runtime_key}:{role_name}")
}

pub(super) struct SlotHandle {
    pub(super) conn: tokio::sync::Mutex<Option<LiveConnection>>,
    pub(super) cancel_flag: std::sync::atomic::AtomicBool,
}

static SLOT_MAP: OnceLock<Arc<DashMap<String, Arc<SlotHandle>>>> = OnceLock::new();

fn slot_map() -> &'static Arc<DashMap<String, Arc<SlotHandle>>> {
    SLOT_MAP.get_or_init(|| Arc::new(DashMap::new()))
}

fn get_slot_handle(runtime_key: &str, role_name: &str) -> Arc<SlotHandle> {
    let key = pool_key(runtime_key, role_name);
    slot_map()
        .entry(key)
        .or_insert_with(|| {
            Arc::new(SlotHandle {
                conn: tokio::sync::Mutex::new(None),
                cancel_flag: std::sync::atomic::AtomicBool::new(false),
            })
        })
        .clone()
}

pub fn reset_slot(runtime_key: &str, role_name: &str) {
    let key = pool_key(runtime_key, role_name);
    let _ = slot_map().remove(&key);
}

pub(super) struct LiveConnection {
    pub(super) conn: acp::ClientSideConnection,
    pub(super) session_id: acp::SessionId,
    pub(super) cwd: String,
    pub(super) delta_slot: DeltaSlot,
    #[allow(dead_code)]
    pub(super) available_modes: Vec<Value>,
    #[allow(dead_code)]
    pub(super) current_mode: Option<String>,
    pub(super) _child: tokio::process::Child,
    pub(super) _io_task: tokio::task::JoinHandle<()>,
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
                prompt,
                context,
                cwd,
                delta_tx,
                result_tx,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    handle_execute(
                        slot,
                        runtime_key,
                        binary,
                        args,
                        env,
                        role_name,
                        prompt,
                        context,
                        cwd,
                        delta_tx,
                        result_tx,
                        auto_approve,
                        mcp_servers,
                        role_mode,
                        role_config_options,
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
                cwd,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
                result_tx,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name);
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
                    )
                    .await;
                });
            }
            WorkerMsg::Cancel {
                runtime_key,
                role_name,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name);
                slot.cancel_flag
                    .store(true, std::sync::atomic::Ordering::Release);
                tokio::task::spawn_local(async move {
                    let guard = slot.conn.lock().await;
                    if let Some(live) = guard.as_ref() {
                        let _ = live
                            .conn
                            .cancel(acp::CancelNotification::new(live.session_id.clone()))
                            .await;
                    }
                });
            }
            WorkerMsg::SetMode {
                runtime_key,
                role_name,
                mode_id,
                result_tx,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name);
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
                config_id,
                value,
                result_tx,
            } => {
                let slot = get_slot_handle(runtime_key, &role_name);
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
        }
    }
}

async fn handle_execute(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    prompt: String,
    context: Vec<(String, String)>,
    cwd: String,
    delta_tx: mpsc::UnboundedSender<AcpEvent>,
    result_tx: oneshot::Sender<Result<(String, String), String>>,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
) {
    slot.cancel_flag
        .store(false, std::sync::atomic::Ordering::Release);
    let mut guard = slot.conn.lock().await;
    let resolved = resolve_cwd(&cwd);

    if guard.as_ref().map(|c| c.cwd != resolved).unwrap_or(false) {
        acp_log(
            "pool.evict",
            json!({ "runtime": runtime_key, "role": role_name, "reason": "cwd_change" }),
        );
        *guard = None;
    }

    let is_cold = guard.is_none();
    if is_cold {
        match cold_start(
            runtime_key,
            &binary,
            &args,
            &env,
            &resolved,
            auto_approve,
            mcp_servers,
        )
        .await
        {
            Ok(conn) => {
                acp_log(
                    "pool.cold_start",
                    json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
                );
                *guard = Some(conn);
            }
            Err(e) => {
                let _ = result_tx.send(Err(e));
                return;
            }
        }
    } else {
        acp_log(
            "pool.reuse",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
    }

    let conn = guard.as_mut().unwrap();
    let session_id = conn.session_id.clone();

    if let Ok(mut slot) = conn.delta_slot.lock() {
        *slot = Some(delta_tx.clone());
    }

    if is_cold {
        if !conn.available_modes.is_empty() {
            let _ = delta_tx.send(AcpEvent::AvailableModes {
                modes: conn.available_modes.clone(),
                current: conn.current_mode.clone(),
            });
        }

        if let Some(mode) = &role_mode {
            let _ = conn
                .conn
                .set_session_mode(acp::SetSessionModeRequest::new(
                    session_id.clone(),
                    acp::SessionModeId::from(mode.clone()),
                ))
                .await;
        }
        for (key, value) in &role_config_options {
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

    let prompt_started = Instant::now();
    let prompt_result = conn
        .conn
        .prompt(acp::PromptRequest::new(session_id.clone(), blocks))
        .await;

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
    result_tx: Option<oneshot::Sender<Vec<Value>>>,
) {
    let mut guard = slot.conn.lock().await;
    if guard.is_some() {
        if let Some(tx) = result_tx {
            let _ = tx.send(list_discovered_config_options(runtime_key));
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
    )
    .await
    {
        Ok(conn) => {
            acp_log(
                "prewarm.ok",
                json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
            );
            let session_id = conn.session_id.clone();
            if let Some(mode) = &role_mode {
                let _ = conn
                    .conn
                    .set_session_mode(acp::SetSessionModeRequest::new(
                        session_id.clone(),
                        acp::SessionModeId::from(mode.clone()),
                    ))
                    .await;
            }
            for (key, value) in &role_config_options {
                let _ = conn
                    .conn
                    .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                        session_id.clone(),
                        acp::SessionConfigId::from(key.clone()),
                        acp::SessionConfigValueId::from(value.clone()),
                    ))
                    .await;
            }
            *guard = Some(conn);
            if let Some(tx) = result_tx {
                let _ = tx.send(list_discovered_config_options(runtime_key));
            }
        }
        Err(e) => {
            acp_log(
                "prewarm.error",
                json!({ "runtime": runtime_key, "role": role_name, "error": e }),
            );
            if let Some(tx) = result_tx {
                let _ = tx.send(vec![]);
            }
        }
    }
}
