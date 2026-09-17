//! Unified `RuntimeAdapter` lifecycle contract.
//!
//! Every runtime resolves to exactly one adapter (see [`AnyRuntimeAdapter`]).
//! The adapter owns the provider protocol boundary and the conversion of
//! provider events into the canonical `AcpEvent` envelope; process slots,
//! backpressure, event sequencing, stderr tails, and idle eviction stay with
//! the existing per-transport runners until they are folded into the shared
//! session runner. The trait fixes the entry points so callers (UI commands,
//! execute dispatch, app teardown) never branch on transport kind.

use serde_json::{json, Value};

use super::super::adapter::{AdapterTransport, HeadlessProtocol, NativeProtocol};
use super::super::protocol as acp;
use super::super::worker::{worker_tx, AcpPromptResult, WorkerMsg};
use super::execute::{mock_execute, AcpWorkerPromptContext};
use super::headless::{cancel_headless, discard_headless_session, execute_headless_runtime};
use super::native::{
    cancel_native, discard_native_session, execute_native_runtime, NativeRunRequest,
};
use crate::runtime_profile::RuntimeCapabilities;

/// Identity of a per-session provider slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionKey {
    pub(crate) runtime_key: String,
    pub(crate) role_name: String,
    pub(crate) app_session_id: String,
}

impl SessionKey {
    pub(crate) fn new(
        runtime_key: impl Into<String>,
        role_name: impl Into<String>,
        app_session_id: impl Into<String>,
    ) -> Self {
        Self {
            runtime_key: runtime_key.into(),
            role_name: role_name.into(),
            app_session_id: app_session_id.into(),
        }
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.app_session_id, self.runtime_key, self.role_name
        )
    }
}

/// Result of probing an adapter without session side effects.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct AdapterProbe {
    pub(crate) available: bool,
    pub(crate) launch_method: Option<String>,
    pub(crate) transport: Option<String>,
    /// Unavailable reason, or resolved binary info when available.
    pub(crate) detail: Option<String>,
}

/// Everything one turn needs. Built by the execute dispatcher after adapter
/// resolution; adapters never re-resolve binaries mid-turn.
pub(crate) struct PromptRequest<'a> {
    pub(crate) runtime_key: &'static str,
    pub(crate) role_name: &'a str,
    pub(crate) app_session_id: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) context: &'a [(String, String)],
    pub(crate) attachments: &'a [crate::types::ImageAttachment],
    pub(crate) cwd: &'a str,
    pub(crate) app: &'a tauri::AppHandle,
    pub(crate) auto_approve: bool,
    pub(crate) role_mode: Option<&'a str>,
    pub(crate) role_config_options: &'a [(String, String)],
    pub(crate) transport: AdapterTransport,
    pub(crate) agent_kind: crate::runtime_kind::RuntimeKind,
    pub(crate) binary: &'a str,
    pub(crate) adapter_args: &'a [String],
    pub(crate) env: &'a [(String, String)],
    pub(crate) resume_session_id: Option<&'a str>,
    pub(crate) mcp_servers: &'a [acp::McpServer],
}

/// Fixed lifecycle entry points shared by every runtime.
pub(crate) trait RuntimeAdapter {
    /// Stable transport label (diagnostics / launch method surface).
    fn transport_name(&self) -> &'static str;

    /// Binary availability + capability gating, without touching slots.
    /// Consumed by the runtime diagnostics surface once it wires into this
    /// trait instead of the assistant catalog probe.
    #[allow(dead_code)]
    fn probe(&self) -> AdapterProbe;

    /// Execute one turn; slot reuse and cold-start resume are internal.
    async fn prompt(&self, request: PromptRequest<'_>) -> AcpPromptResult;

    /// Cancel the in-flight turn for `key`; true when a slot was known.
    fn cancel(&self, key: &SessionKey) -> bool;

    /// Discard the provider session slot; the next turn cold-starts and
    /// resumes from the persisted provider handle.
    async fn discard_slot(&self, key: &SessionKey) -> Result<(), String>;

    /// Rebuild the slot. Defaults to a discard; connection-oriented runtimes
    /// (ACP) re-run their handshake instead.
    async fn reconnect_slot(&self, key: &SessionKey) -> Result<(), String> {
        self.discard_slot(key).await
    }

    /// Tear down every slot for this runtime (app teardown).
    async fn teardown(&self);

    /// Evict slots idle past their budget.
    fn reclaim_idle(&self);

    /// What this transport can actually surface. The UI degrades on this instead of
    /// guessing: Antigravity's print mode, for instance, reports no tool detail at all,
    /// so rendering tool cards for it produces empty rows.
    #[allow(dead_code)]
    fn capabilities(&self) -> RuntimeCapabilities {
        crate::runtime_profile::capabilities_for_transport(self.transport_name())
    }

    /// Switch mode on live session (e.g. ACP plan/act). Defaults to no-op for
    /// runtimes without live mode switching.
    async fn set_mode(&self, key: &SessionKey, mode_id: &str) -> Result<(), String> {
        let _ = (key, mode_id);
        Ok(())
    }

    /// Set config option on live session. Defaults to no-op for runtimes
    /// without live config switching.
    async fn set_config_option(
        &self,
        key: &SessionKey,
        option_id: &str,
        value: &str,
    ) -> Result<(), String> {
        let _ = (key, option_id, value);
        Ok(())
    }

    /// Diagnostic snapshot for the runtime list. Consumed by the runtime
    /// diagnostics surface once it wires into this trait.
    #[allow(dead_code)]
    fn diagnostics(&self) -> Value {
        json!({ "transport": self.transport_name() })
    }
}

/// Concrete adapter variants. An enum keeps dispatch static and mirrors the
/// existing transport resolution; a single runtime key maps to exactly one
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnyRuntimeAdapter {
    Headless { protocol: HeadlessProtocol },
    Native { protocol: NativeProtocol },
    AcpWorker,
    Mock,
}

impl AnyRuntimeAdapter {
    /// Every concrete adapter variant, for lifecycle sweeps that must cover
    /// all runtimes (idle eviction, app teardown).
    pub(crate) fn all() -> [Self; 6] {
        [
            Self::Headless {
                protocol: HeadlessProtocol::AgyStreamJson,
            },
            Self::Headless {
                protocol: HeadlessProtocol::ClaudeStreamJson,
            },
            Self::Native {
                protocol: NativeProtocol::CodexAppServer,
            },
            Self::Native {
                protocol: NativeProtocol::PiRpc,
            },
            Self::AcpWorker,
            Self::Mock,
        ]
    }
}

impl AnyRuntimeAdapter {
    /// Derive the adapter variant directly from a resolved transport without
    /// re-probing the adapter cache or PATH.
    pub(crate) fn from_transport(transport: &AdapterTransport) -> Self {
        match transport {
            AdapterTransport::HeadlessJson { protocol, .. } => Self::Headless {
                protocol: *protocol,
            },
            AdapterTransport::Native(protocol) => Self::Native {
                protocol: *protocol,
            },
            AdapterTransport::Acp => Self::AcpWorker,
        }
    }

    /// Resolve the adapter for a runtime key via the existing transport
    /// resolution (binary probing is cached there).
    pub(crate) fn resolve(runtime_kind: &str) -> Option<Self> {
        let normalized = runtime_kind.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized == "mock" {
            return Some(Self::Mock);
        }
        match super::super::adapter::build_stdio_adapter(&normalized) {
            Ok(Some(spec)) => Some(Self::from_transport(&spec.transport)),
            Ok(None) => Some(Self::Mock),
            Err(_) => None,
        }
    }
}

impl RuntimeAdapter for AnyRuntimeAdapter {
    fn transport_name(&self) -> &'static str {
        match self {
            Self::Headless { protocol } => {
                crate::acp::session::headless::protocol_transport_name(*protocol)
            }
            Self::Native { protocol } => protocol_display_name_for(*protocol),
            Self::AcpWorker => "acp",
            Self::Mock => "mock",
        }
    }

    fn probe(&self) -> AdapterProbe {
        let runtime_key = match self {
            Self::Headless { protocol } => match protocol {
                HeadlessProtocol::AgyStreamJson => "antigravity-cli",
                HeadlessProtocol::ClaudeStreamJson => "claude-native",
            },
            Self::Native { protocol } => match protocol {
                NativeProtocol::CodexAppServer => "codex-cli",
                NativeProtocol::PiRpc => "pi-cli",
            },
            Self::AcpWorker => "claude-code",
            Self::Mock => "mock",
        };
        match super::super::adapter::build_stdio_adapter(runtime_key) {
            Ok(Some(spec)) => AdapterProbe {
                available: true,
                launch_method: Some(spec.launch_method),
                transport: Some(self.transport_name().to_string()),
                detail: Some(spec.binary),
            },
            Ok(None) => AdapterProbe {
                available: false,
                launch_method: None,
                transport: Some(self.transport_name().to_string()),
                detail: Some("unsupported runtime kind".to_string()),
            },
            Err(error) => AdapterProbe {
                available: false,
                launch_method: None,
                transport: Some(self.transport_name().to_string()),
                detail: Some(error),
            },
        }
    }

    async fn prompt(&self, request: PromptRequest<'_>) -> AcpPromptResult {
        match self {
            Self::Headless { .. } => execute_headless_runtime(&request).await,
            Self::Native { protocol } => {
                execute_native_runtime(NativeRunRequest {
                    protocol: *protocol,
                    runtime_key: request.runtime_key,
                    role_name: request.role_name,
                    prompt: request.prompt,
                    context: request.context,
                    attachments: request.attachments,
                    cwd: request.cwd,
                    app: request.app,
                    auto_approve: request.auto_approve,
                    role_mode: request.role_mode,
                    role_config_options: request.role_config_options,
                    binary: request.binary,
                    adapter_args: request.adapter_args,
                    env: request.env,
                    resume_session_id: request.resume_session_id,
                    mcp_servers: request.mcp_servers,
                    app_session_id: request.app_session_id,
                })
                .await
            }
            // The ACP worker owns the shared delta-batching/heartbeat runner;
            // its prompt path is delegated unchanged (see execute.rs).
            Self::AcpWorker => {
                let context = AcpWorkerPromptContext {
                    runtime_key: request.runtime_key,
                    role_name: request.role_name.to_string(),
                    app_session_id: request.app_session_id.to_string(),
                    agent_kind: request.agent_kind,
                    binary: request.binary.to_string(),
                    args: request.adapter_args.to_vec(),
                    env: request.env.to_vec(),
                    prompt: request.prompt.to_string(),
                    context: request.context.to_vec(),
                    attachments: request.attachments.to_vec(),
                    cwd: request.cwd.to_string(),
                    auto_approve: request.auto_approve,
                    mcp_servers: request.mcp_servers.to_vec(),
                    role_mode: request.role_mode.map(str::to_string),
                    role_config_options: request.role_config_options.to_vec(),
                    resume_session_id: request.resume_session_id.map(str::to_string),
                };
                super::execute::execute_acp_worker_prompt(context, request.app.clone()).await
            }
            Self::Mock => mock_execute(request.role_name, request.prompt, request.context),
        }
    }

    fn cancel(&self, key: &SessionKey) -> bool {
        let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
            return false;
        };
        match self {
            Self::Headless { .. } => {
                cancel_headless(runtime_key, &key.role_name, &key.app_session_id)
            }
            Self::Native { .. } => cancel_native(runtime_key, &key.role_name, &key.app_session_id),
            // Fire-and-forget: callers that need the drained confirmation use
            // the cancel_session command path, which awaits the worker oneshot.
            Self::AcpWorker => {
                let _ = worker_tx().send(WorkerMsg::Cancel {
                    runtime_key,
                    role_name: key.role_name.clone(),
                    app_session_id: key.app_session_id.clone(),
                    result_tx: None,
                });
                true
            }
            Self::Mock => false,
        }
    }

    async fn discard_slot(&self, key: &SessionKey) -> Result<(), String> {
        let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
            return Err(format!("unsupported runtime: {}", key.runtime_key));
        };
        match self {
            Self::Headless { .. } => {
                discard_headless_session(runtime_key, &key.role_name, &key.app_session_id).await
            }
            Self::Native { .. } => {
                discard_native_session(runtime_key, &key.role_name, &key.app_session_id).await
            }
            Self::AcpWorker | Self::Mock => {
                acp_worker_reset(runtime_key, key, WorkerMsgKind::Reset).await
            }
        }
    }

    async fn reconnect_slot(&self, key: &SessionKey) -> Result<(), String> {
        let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
            return Err(format!("unsupported runtime: {}", key.runtime_key));
        };
        match self {
            Self::AcpWorker => acp_worker_reset(runtime_key, key, WorkerMsgKind::Reconnect).await,
            _ => RuntimeAdapter::discard_slot(self, key).await,
        }
    }

    async fn teardown(&self) {
        match self {
            Self::Headless { .. } => {
                super::headless::shutdown_headless_sessions().await;
            }
            Self::Native { .. } => {
                super::native::shutdown_native_sessions().await;
            }
            Self::AcpWorker | Self::Mock => {
                // Worker teardown is owned by acp::shutdown (worker loop stop).
            }
        }
    }

    fn reclaim_idle(&self) {
        match self {
            Self::Headless { .. } => super::headless::reclaim_idle_headless_sessions(),
            Self::Native { .. } => super::native::reclaim_idle_native_sessions(),
            Self::AcpWorker | Self::Mock => {}
        }
    }

    async fn set_mode(&self, key: &SessionKey, mode_id: &str) -> Result<(), String> {
        match self {
            Self::AcpWorker => {
                let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
                    return Err(format!("unsupported runtime: {}", key.runtime_key));
                };
                let (tx, rx) = tokio::sync::oneshot::channel();
                if worker_tx()
                    .send(WorkerMsg::SetMode {
                        runtime_key,
                        role_name: key.role_name.clone(),
                        app_session_id: key.app_session_id.clone(),
                        mode_id: mode_id.to_string(),
                        result_tx: tx,
                    })
                    .is_err()
                {
                    return Err("worker channel closed".to_string());
                }
                rx.await.map_err(|_| "worker disconnected".to_string())?
            }
            _ => Ok(()),
        }
    }

    async fn set_config_option(
        &self,
        key: &SessionKey,
        option_id: &str,
        value: &str,
    ) -> Result<(), String> {
        match self {
            Self::AcpWorker => {
                let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
                    return Err(format!("unsupported runtime: {}", key.runtime_key));
                };
                let (tx, rx) = tokio::sync::oneshot::channel();
                if worker_tx()
                    .send(WorkerMsg::SetConfigOption {
                        runtime_key,
                        role_name: key.role_name.clone(),
                        app_session_id: key.app_session_id.clone(),
                        config_id: option_id.to_string(),
                        value: value.to_string(),
                        result_tx: tx,
                    })
                    .is_err()
                {
                    return Err("worker channel closed".to_string());
                }
                rx.await.map_err(|_| "worker disconnected".to_string())?
            }
            _ => Ok(()),
        }
    }
}

/// Which worker reset flavour to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerMsgKind {
    Reset,
    Reconnect,
}

/// Normalize a runtime key to its `'static` form (builtin kinds resolve via
/// `RuntimeKind`; custom ACP profiles leak-intern their key).
#[allow(dead_code)]
fn protocol_display_name_for(protocol: NativeProtocol) -> &'static str {
    match protocol {
        NativeProtocol::CodexAppServer => "native-codex-app-server",
        NativeProtocol::PiRpc => "native-pi-rpc",
    }
}

fn static_runtime_key(runtime_key: &str) -> Option<&'static str> {
    crate::runtime_kind::RuntimeKind::from_str(runtime_key)
        .map(|kind| kind.runtime_key())
        .or_else(|| crate::runtime_profile::runtime_key_static(runtime_key))
}

async fn acp_worker_reset(
    runtime_key: &'static str,
    key: &SessionKey,
    kind: WorkerMsgKind,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sent = match kind {
        WorkerMsgKind::Reset => worker_tx().send(WorkerMsg::Reset {
            runtime_key,
            role_name: key.role_name.clone(),
            app_session_id: key.app_session_id.clone(),
            result_tx: tx,
        }),
        WorkerMsgKind::Reconnect => worker_tx().send(WorkerMsg::Reconnect {
            runtime_key,
            role_name: key.role_name.clone(),
            app_session_id: key.app_session_id.clone(),
            result_tx: tx,
        }),
    };
    if sent.is_err() {
        return Err("worker disconnected".to_string());
    }
    rx.await.map_err(|_| "worker disconnected".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SessionKey {
        SessionKey::new("claude-native", "reviewer", "sess-1")
    }

    /// A fake adapter exercising the trait contract without any provider
    /// process; this is the seam future transport fakes plug into.
    struct FakeAdapter {
        probe_result: AdapterProbe,
        cancelled: std::sync::Mutex<Vec<SessionKey>>,
    }

    impl RuntimeAdapter for FakeAdapter {
        fn transport_name(&self) -> &'static str {
            "fake"
        }

        fn probe(&self) -> AdapterProbe {
            self.probe_result.clone()
        }

        async fn prompt(&self, _request: PromptRequest<'_>) -> AcpPromptResult {
            AcpPromptResult {
                ok: true,
                output: "fake".to_string(),
                error_code: None,
                deltas: vec![],
                meta: json!({}),
                session_handle: None,
            }
        }

        fn cancel(&self, key: &SessionKey) -> bool {
            self.cancelled.lock().unwrap().push(key.clone());
            true
        }

        async fn discard_slot(&self, _key: &SessionKey) -> Result<(), String> {
            Ok(())
        }

        async fn teardown(&self) {}

        fn reclaim_idle(&self) {}
    }

    fn fake_adapter() -> FakeAdapter {
        FakeAdapter {
            probe_result: AdapterProbe {
                available: true,
                launch_method: Some("fake-launch".to_string()),
                transport: Some("fake".to_string()),
                detail: Some("/fake/binary".to_string()),
            },
            cancelled: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn probe_reports_availability_and_detail() {
        let adapter = fake_adapter();
        let probe = RuntimeAdapter::probe(&adapter);
        assert!(probe.available);
        assert_eq!(probe.launch_method.as_deref(), Some("fake-launch"));
        assert_eq!(probe.detail.as_deref(), Some("/fake/binary"));
    }

    #[tokio::test]
    async fn cancel_routes_session_keys() {
        let adapter = fake_adapter();
        let key = key();
        assert!(RuntimeAdapter::cancel(&adapter, &key));
        let cancelled = adapter.cancelled.lock().unwrap();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0], key);
    }

    #[tokio::test]
    async fn reconnect_defaults_to_discard() {
        let adapter = fake_adapter();
        // Reconnect on a slot-based fake must succeed through the default
        // discard path.
        assert!(RuntimeAdapter::reconnect_slot(&adapter, &key())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn diagnostics_includes_transport() {
        let adapter = fake_adapter();
        let diagnostics = RuntimeAdapter::diagnostics(&adapter);
        assert_eq!(diagnostics["transport"], "fake");
    }

    #[test]
    fn session_key_display_matches_headless_key_layout() {
        let rendered = key().to_string();
        assert_eq!(rendered, "sess-1:claude-native:reviewer");
        let parsed = super::super::headless::headless_key("claude-native", "reviewer", "sess-1");
        assert_eq!(rendered, parsed);
    }

    #[test]
    fn from_transport_maps_variants_correctly() {
        assert_eq!(
            AnyRuntimeAdapter::from_transport(&AdapterTransport::Acp),
            AnyRuntimeAdapter::AcpWorker
        );
        assert_eq!(
            AnyRuntimeAdapter::from_transport(&AdapterTransport::Native(
                NativeProtocol::CodexAppServer
            )),
            AnyRuntimeAdapter::Native {
                protocol: NativeProtocol::CodexAppServer
            }
        );
        assert_eq!(
            AnyRuntimeAdapter::from_transport(&AdapterTransport::HeadlessJson {
                protocol: HeadlessProtocol::ClaudeStreamJson,
                stream_input: true,
                output_format: true,
                conversation: true,
            }),
            AnyRuntimeAdapter::Headless {
                protocol: HeadlessProtocol::ClaudeStreamJson
            }
        );
    }
}
