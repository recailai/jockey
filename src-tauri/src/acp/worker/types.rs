use crate::acp::protocol as acp;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

// ── Event types broadcast to the frontend ────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDeathEvent {
    pub runtime_key: String,
    pub role_name: String,
    pub app_session_id: String,
    pub reason: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrewarmEvent {
    pub runtime_key: String,
    pub role_name: String,
    pub app_session_id: String,
    pub status: PrewarmStatus,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveConnectionInfo {
    pub key: String,
    pub runtime_key: String,
    pub role_name: String,
    pub app_session_id: String,
    pub acp_session_id: String,
    pub cwd: String,
    pub child_pid: Option<u32>,
    pub idle_ms: u128,
    pub healthy: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrewarmStatus {
    Started,
    Ready,
    Failed { error: String },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPromptResult {
    pub ok: bool,
    pub output: String,
    pub error_code: Option<String>,
    pub deltas: Vec<String>,
    pub meta: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_handle: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
        terminal_meta: Option<Value>,
        /// Owning tool call, when the provider reports nesting (sub-agents, spawned tasks).
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<Value>,
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
        terminal_meta: Option<Value>,
        /// Owning tool call, when the provider reports nesting (sub-agents, spawned tasks).
        /// `None` keeps the call at the top level.
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        /// Structured patch for file-editing tools, so a diff can be rendered without
        /// re-parsing `raw_output`.
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<Value>,
    },
    /// Incremental output from a long-running tool. Separate from `ToolCallUpdate` so a
    /// command's stdout can stream without resending the whole call on every chunk.
    ToolOutputDelta {
        tool_call_id: String,
        delta: String,
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
    PermissionExpired {
        request_id: String,
    },
    /// A structured question set the agent needs answered before it can continue.
    /// Distinct from `PermissionRequest`, which is only allow/deny on a single action:
    /// this can carry several questions, each multi-select, free-text or secret.
    UserInputRequest {
        request_id: String,
        title: Option<String>,
        blocking: bool,
        questions: Vec<Value>,
    },
    /// The provider compacted the conversation. Worth surfacing because it silently
    /// changes what the agent can still recall.
    ContextCompacted {
        reason: Option<String>,
        before_tokens: Option<u64>,
        after_tokens: Option<u64>,
    },
    /// Token accounting for the turn. Every supported CLI reports this in some form;
    /// fields the provider does not supply stay `None` rather than being zeroed, so the UI
    /// can tell "not reported" apart from "zero".
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        total_tokens: Option<u64>,
        /// Model context window, when the provider reports it — lets the UI show pressure.
        context_window: Option<u64>,
        cost_usd: Option<f64>,
    },
    /// Out-of-band advisory from the provider: rate limits, retries, model reroutes,
    /// deprecation warnings. Distinct from `SessionError`, which aborts the turn.
    Notice {
        level: String,
        code: Option<String>,
        text: String,
    },
    /// Structured error notification surfaced to the UI before the final
    /// execute-promise rejection. Consumers route on `code` (the
    /// `AcpErrorCode::as_str()` value) to pick a recovery action.
    SessionError {
        code: String,
        message: String,
        retryable: bool,
    },
}

// ── Worker message channel ────────────────────────────────────────────────────

pub(crate) enum WorkerMsg {
    Execute {
        runtime_key: &'static str,
        binary: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        role_name: String,
        app_session_id: String,
        prompt: String,
        context: Vec<(String, String)>,
        attachments: Vec<crate::types::ImageAttachment>,
        cwd: String,
        delta_tx: mpsc::Sender<AcpEvent>,
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
        result_tx: Option<oneshot::Sender<(Vec<Value>, Vec<String>, String)>>,
        resume_session_id: Option<String>,
        force_refresh: bool,
    },
    Cancel {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
        /// Resolved once the old prompt has released its PROMPT_LOCK
        /// (i.e. agent responded with StopReason::Cancelled or process died).
        /// None = fire-and-forget (legacy callers).
        result_tx: Option<oneshot::Sender<()>>,
    },
    Reset {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
        result_tx: oneshot::Sender<Result<(), String>>,
    },
    Reconnect {
        runtime_key: &'static str,
        role_name: String,
        app_session_id: String,
        result_tx: oneshot::Sender<Result<(), String>>,
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
    SyncRoleMode {
        role_name: String,
        mode_id: String,
        eligible_session_ids: Vec<String>,
        result_tx: oneshot::Sender<Vec<String>>,
    },
    Shutdown {
        done_tx: oneshot::Sender<()>,
    },
    SnapshotConnections {
        result_tx: oneshot::Sender<Vec<ActiveConnectionInfo>>,
    },
}
