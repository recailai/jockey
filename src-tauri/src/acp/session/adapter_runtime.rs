//! Unified `RuntimeAdapter` lifecycle contract.
//!
//! All real agents run through the shared ACP worker. Provider-specific wire
//! conversion belongs to the external ACP adapters; the host owns session
//! slots, cancellation, event delivery and process lifetime.

use serde::Serialize;
use serde_json::{json, Value};

use super::super::adapter::AdapterTransport;
use super::super::protocol as acp;
use super::super::worker::{worker_tx, AcpPromptResult, SteerOutcome, WorkerMsg};
use super::execute::{mock_execute, AcpWorkerPromptContext};
use crate::runtime_profile::RuntimeCapabilities;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdapterError {
    UnsupportedCapability(&'static str),
    UnsupportedRuntime(String),
    Unavailable(String),
    Failed(String),
}

impl AdapterError {
    pub(crate) fn unsupported(capability: &'static str) -> Self {
        Self::UnsupportedCapability(capability)
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedCapability(capability) => {
                write!(f, "UnsupportedCapability({capability})")
            }
            Self::UnsupportedRuntime(runtime) => write!(f, "unsupported runtime: {runtime}"),
            Self::Unavailable(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<String> for AdapterError {
    fn from(value: String) -> Self {
        Self::Failed(value)
    }
}

impl From<&str> for AdapterError {
    fn from(value: &str) -> Self {
        Self::Failed(value.to_string())
    }
}

impl From<AdapterError> for String {
    fn from(value: AdapterError) -> Self {
        value.to_string()
    }
}

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

/// Steer error text for "the turn ended before the input arrived"; the engine drains the queue
/// normally instead of interrupting anything.
pub(crate) const STEER_NO_RUNNING_TURN: &str = "no running turn to steer";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputDeliveryCapabilities {
    pub(crate) next_turn: bool,
    pub(crate) next_step: bool,
    pub(crate) interrupt: bool,
    pub(crate) run_now: bool,
    pub(crate) strategy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdapterDescriptor {
    pub(crate) runtime_key: String,
    pub(crate) transport: String,
    pub(crate) capabilities: RuntimeCapabilities,
    pub(crate) input_delivery: InputDeliveryCapabilities,
}

impl AdapterDescriptor {
    fn from_adapter(runtime_key: &str, transport: &str, capabilities: RuntimeCapabilities) -> Self {
        let strategy = match transport {
            _ if capabilities.streaming => "interruptFollowUp",
            _ => "queue",
        };
        let next_step = false;
        Self {
            runtime_key: runtime_key.to_string(),
            transport: transport.to_string(),
            input_delivery: InputDeliveryCapabilities {
                next_turn: capabilities.streaming,
                next_step,
                interrupt: capabilities.streaming,
                run_now: capabilities.streaming,
                strategy: strategy.to_string(),
            },
            capabilities,
        }
    }
}

/// Everything one turn needs. Built by the execute dispatcher after adapter
/// resolution; adapters never re-resolve binaries mid-turn.
pub(crate) struct PromptRequest<'a> {
    pub(crate) runtime_key: &'static str,
    pub(crate) role_name: &'a str,
    pub(crate) app_session_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) attachments: &'a [crate::types::ImageAttachment],
    pub(crate) cwd: &'a str,
    pub(crate) app: &'a tauri::AppHandle,
    pub(crate) auto_approve: bool,
    pub(crate) role_mode: Option<&'a str>,
    pub(crate) role_config_options: &'a [(String, String)],
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

    fn runtime_key(&self) -> &'static str;

    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor::from_adapter(
            self.runtime_key(),
            self.transport_name(),
            self.capabilities(),
        )
    }

    /// Binary availability + capability gating, without touching slots.
    /// Consumed by the runtime diagnostics surface once it wires into this
    /// trait instead of the assistant catalog probe.
    #[allow(dead_code)]
    fn probe(&self) -> AdapterProbe;

    /// Execute one turn; slot reuse and cold-start resume are internal.
    async fn prompt(&self, request: PromptRequest<'_>) -> AcpPromptResult;

    /// Request cancellation for `key`; an idle or missing slot is idempotent success.
    fn cancel(&self, key: &SessionKey) -> Result<(), AdapterError>;

    /// Deliver input to an active turn without opening a second prompt. Providers without a
    /// concurrent control channel use the explicit unsupported-capability result so callers can
    /// fall back to cancel-and-drain without guessing from transport names.
    async fn steer(
        &self,
        key: &SessionKey,
        prompt: &str,
        attachments: &[crate::types::ImageAttachment],
    ) -> Result<(), AdapterError> {
        let _ = (key, prompt, attachments);
        Err(AdapterError::unsupported("steer"))
    }

    /// Discard the provider session slot; the next turn cold-starts and
    /// resumes from the persisted provider handle.
    async fn discard_slot(&self, key: &SessionKey) -> Result<(), AdapterError>;

    /// Rebuild the slot. Defaults to a discard; connection-oriented runtimes
    /// (ACP) re-run their handshake instead.
    async fn reconnect_slot(&self, key: &SessionKey) -> Result<(), AdapterError> {
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

    /// Switch mode on live session (e.g. ACP plan/act).
    async fn set_mode(&self, key: &SessionKey, mode_id: &str) -> Result<(), AdapterError> {
        let _ = (key, mode_id);
        Err(AdapterError::unsupported("dynamicModes"))
    }

    /// Set config option on live session.
    async fn set_config_option(
        &self,
        key: &SessionKey,
        option_id: &str,
        value: &str,
    ) -> Result<(), AdapterError> {
        let _ = (key, option_id, value);
        Err(AdapterError::unsupported("dynamicConfig"))
    }

    /// Diagnostic snapshot for the runtime list. Consumed by the runtime
    /// diagnostics surface once it wires into this trait.
    #[allow(dead_code)]
    fn diagnostics(&self) -> Value {
        json!({ "transport": self.transport_name() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnyRuntimeAdapter {
    AcpWorker { runtime_key: &'static str },
    Mock,
}

impl AnyRuntimeAdapter {
    pub(crate) fn all() -> [Self; 2] {
        [
            Self::AcpWorker {
                runtime_key: "claude-code",
            },
            Self::Mock,
        ]
    }
    pub(crate) fn from_transport(_: &AdapterTransport, runtime_key: &'static str) -> Self {
        Self::AcpWorker { runtime_key }
    }
    pub(crate) fn resolve(runtime: &str) -> Option<Self> {
        if runtime.trim().is_empty() || runtime == "mock" {
            return Some(Self::Mock);
        }
        crate::runtime_profile::runtime_key_static(runtime)
            .map(|runtime_key| Self::AcpWorker { runtime_key })
    }
}

impl RuntimeAdapter for AnyRuntimeAdapter {
    fn transport_name(&self) -> &'static str {
        match self {
            Self::AcpWorker { .. } => "acp",
            Self::Mock => "mock",
        }
    }
    fn runtime_key(&self) -> &'static str {
        match self {
            Self::AcpWorker { runtime_key } => runtime_key,
            Self::Mock => "mock",
        }
    }
    fn probe(&self) -> AdapterProbe {
        let runtime = self.runtime_key();
        let (available, detail) = super::super::adapter::probe_runtime(runtime)
            .unwrap_or((false, "ACP runtime unavailable".into()));
        AdapterProbe {
            available,
            launch_method: super::super::adapter::adapter_launch_method(runtime),
            transport: Some(self.transport_name().into()),
            detail: Some(detail),
        }
    }
    async fn prompt(&self, request: PromptRequest<'_>) -> AcpPromptResult {
        match self {
            Self::AcpWorker { .. } => {
                let context = AcpWorkerPromptContext {
                    runtime_key: request.runtime_key,
                    role_name: request.role_name.to_string(),
                    app_session_id: request.app_session_id.to_string(),
                    turn_id: request.turn_id.to_string(),
                    agent_kind: request.agent_kind,
                    binary: request.binary.to_string(),
                    args: request.adapter_args.to_vec(),
                    env: request.env.to_vec(),
                    prompt: request.prompt.to_string(),
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
            Self::Mock => mock_execute(request.role_name, request.prompt),
        }
    }
    fn cancel(&self, key: &SessionKey) -> Result<(), AdapterError> {
        if *self == Self::Mock {
            return Ok(());
        }
        let runtime_key = static_runtime_key(&key.runtime_key)
            .ok_or_else(|| AdapterError::UnsupportedRuntime(key.runtime_key.clone()))?;
        worker_tx()
            .send(WorkerMsg::Cancel {
                runtime_key,
                role_name: key.role_name.clone(),
                app_session_id: key.app_session_id.clone(),
                result_tx: None,
            })
            .map_err(|_| AdapterError::Unavailable("ACP worker is unavailable".into()))
    }
    async fn steer(
        &self,
        key: &SessionKey,
        prompt: &str,
        attachments: &[crate::types::ImageAttachment],
    ) -> Result<(), AdapterError> {
        if *self == Self::Mock {
            return Err(AdapterError::unsupported("steer"));
        }
        let runtime_key = static_runtime_key(&key.runtime_key)
            .ok_or_else(|| AdapterError::UnsupportedRuntime(key.runtime_key.clone()))?;
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        worker_tx()
            .send(WorkerMsg::Steer {
                runtime_key,
                role_name: key.role_name.clone(),
                app_session_id: key.app_session_id.clone(),
                prompt: crate::acp::worker::build_prompt_blocks(prompt.to_string(), attachments),
                result_tx,
            })
            .map_err(|_| AdapterError::Unavailable("ACP worker is unavailable".into()))?;
        match result_rx.await {
            Ok(SteerOutcome::Injected) => Ok(()),
            Ok(SteerOutcome::Unsupported) => Err(AdapterError::unsupported("steer")),
            Ok(SteerOutcome::NoRunningTurn) => {
                Err(AdapterError::Unavailable(STEER_NO_RUNNING_TURN.into()))
            }
            Ok(SteerOutcome::Failed(e)) => Err(AdapterError::Unavailable(e)),
            Err(_) => Err(AdapterError::Unavailable(
                "ACP worker dropped the steer".into(),
            )),
        }
    }
    async fn discard_slot(&self, key: &SessionKey) -> Result<(), AdapterError> {
        if *self == Self::Mock {
            return Ok(());
        }
        let runtime_key = static_runtime_key(&key.runtime_key)
            .ok_or_else(|| AdapterError::UnsupportedRuntime(key.runtime_key.clone()))?;
        acp_worker_reset(runtime_key, key, WorkerMsgKind::Reset)
            .await
            .map_err(AdapterError::from)
    }
    async fn reconnect_slot(&self, key: &SessionKey) -> Result<(), AdapterError> {
        if *self == Self::Mock {
            return Ok(());
        }
        let runtime_key = static_runtime_key(&key.runtime_key)
            .ok_or_else(|| AdapterError::UnsupportedRuntime(key.runtime_key.clone()))?;
        acp_worker_reset(runtime_key, key, WorkerMsgKind::Reconnect)
            .await
            .map_err(AdapterError::from)
    }
    async fn teardown(&self) {}
    fn reclaim_idle(&self) {}
    async fn set_mode(&self, key: &SessionKey, mode_id: &str) -> Result<(), AdapterError> {
        match self {
            Self::AcpWorker { .. } => {
                let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
                    return Err(AdapterError::UnsupportedRuntime(key.runtime_key.clone()));
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
                    return Err(AdapterError::Unavailable(
                        "worker channel closed".to_string(),
                    ));
                }
                rx.await
                    .map_err(|_| AdapterError::Unavailable("worker disconnected".to_string()))?
                    .map_err(AdapterError::from)
            }
            _ => Err(AdapterError::unsupported("dynamicModes")),
        }
    }

    async fn set_config_option(
        &self,
        key: &SessionKey,
        option_id: &str,
        value: &str,
    ) -> Result<(), AdapterError> {
        match self {
            Self::AcpWorker { .. } => {
                let Some(runtime_key) = static_runtime_key(&key.runtime_key) else {
                    return Err(AdapterError::UnsupportedRuntime(key.runtime_key.clone()));
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
                    return Err(AdapterError::Unavailable(
                        "worker channel closed".to_string(),
                    ));
                }
                rx.await
                    .map_err(|_| AdapterError::Unavailable("worker disconnected".to_string()))?
                    .map_err(AdapterError::from)
            }
            _ => Err(AdapterError::unsupported("dynamicConfig")),
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

        fn runtime_key(&self) -> &'static str {
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

        fn cancel(&self, key: &SessionKey) -> Result<(), AdapterError> {
            self.cancelled.lock().unwrap().push(key.clone());
            Ok(())
        }

        async fn discard_slot(&self, _key: &SessionKey) -> Result<(), AdapterError> {
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
        assert!(RuntimeAdapter::cancel(&adapter, &key).is_ok());
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
    fn descriptor_exposes_runtime_and_input_delivery_contract() {
        let adapter = fake_adapter();
        let descriptor = RuntimeAdapter::descriptor(&adapter);
        assert_eq!(descriptor.runtime_key, "fake");
        assert_eq!(descriptor.transport, "fake");
        assert!(!descriptor.input_delivery.next_turn);
        assert!(!descriptor.input_delivery.interrupt);
    }

    #[test]
    fn all_real_runtimes_use_the_acp_worker() {
        for runtime in [
            "claude-code",
            "claude-native",
            "codex-cli",
            "pi-cli",
            "antigravity-cli",
            "dsh-cli",
        ] {
            let adapter = AnyRuntimeAdapter::resolve(runtime).unwrap();
            assert_eq!(adapter.transport_name(), "acp");
            assert_eq!(
                adapter.runtime_key(),
                crate::runtime_profile::runtime_key_static(runtime).unwrap()
            );
        }
        assert_eq!(
            AnyRuntimeAdapter::AcpWorker {
                runtime_key: "claude-code"
            }
            .descriptor()
            .input_delivery
            .strategy,
            "interruptFollowUp"
        );
    }
}
