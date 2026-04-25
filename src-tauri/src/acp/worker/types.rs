use agent_client_protocol as acp;
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
