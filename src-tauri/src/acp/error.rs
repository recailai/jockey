use agent_client_protocol as acp;

const STDERR_TAIL_LIMIT: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcpErrorCode {
    AuthRequired,
    ConnectionFailed,
    ProcessCrashed,
    PromptTimeout,
    RequestCancelled,
    InvalidRequest,
    InvalidParams,
    MethodNotFound,
    ResourceNotFound,
    InternalError,
    AgentError,
}

impl AcpErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::ConnectionFailed => "CONNECTION_FAILED",
            Self::ProcessCrashed => "PROCESS_CRASHED",
            Self::PromptTimeout => "PROMPT_TIMEOUT",
            Self::RequestCancelled => "ACP_REQ_CANCELLED",
            Self::InvalidRequest => "INVALID_ACP_REQUEST",
            Self::InvalidParams => "ACP_INVALID_PARAMS",
            Self::MethodNotFound => "ACP_METHOD_NOT_FOUND",
            Self::ResourceNotFound => "AGENT_SESSION_NOT_FOUND",
            Self::InternalError => "INTERNAL_ERROR",
            Self::AgentError => "AGENT_ERROR",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AcpLayerError {
    pub(crate) code: AcpErrorCode,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

impl AcpLayerError {
    pub(crate) fn new(code: AcpErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub(crate) fn timeout(operation: &str, seconds: u64) -> Self {
        Self::new(
            AcpErrorCode::PromptTimeout,
            format!("timeout: {operation} exceeded {seconds}s"),
            true,
        )
    }

    pub(crate) fn process_crashed(reason: &str, stderr_tail: &str) -> Self {
        let msg = if stderr_tail.trim().is_empty() {
            format!("agent process exited during {reason}")
        } else {
            format!(
                "agent process exited during {reason}: {}",
                stderr_tail.trim()
            )
        };
        Self::new(AcpErrorCode::ProcessCrashed, msg, true)
    }

    pub(crate) fn connection_closed(message: impl Into<String>) -> Self {
        Self::new(AcpErrorCode::ConnectionFailed, message, true)
    }

    pub(crate) fn into_message(self) -> String {
        format!(
            "{}: {}{}",
            self.code.as_str(),
            self.message,
            if self.retryable { " (retryable)" } else { "" }
        )
    }
}

impl From<acp::Error> for AcpLayerError {
    fn from(err: acp::Error) -> Self {
        let code_num = i32::from(err.code.clone());
        let code = match code_num {
            -32000 => AcpErrorCode::AuthRequired,
            -32800 => AcpErrorCode::RequestCancelled,
            -32600 => AcpErrorCode::InvalidRequest,
            -32601 => AcpErrorCode::MethodNotFound,
            -32602 => AcpErrorCode::InvalidParams,
            -32002 => AcpErrorCode::ResourceNotFound,
            -32603 => {
                if looks_auth_related(&err.message) {
                    AcpErrorCode::AuthRequired
                } else if looks_connection_related(&err.message) {
                    AcpErrorCode::ConnectionFailed
                } else {
                    AcpErrorCode::InternalError
                }
            }
            _ => {
                if looks_auth_related(&err.message) {
                    AcpErrorCode::AuthRequired
                } else if looks_connection_related(&err.message) {
                    AcpErrorCode::ConnectionFailed
                } else {
                    AcpErrorCode::AgentError
                }
            }
        };
        let retryable = matches!(
            code,
            AcpErrorCode::AuthRequired
                | AcpErrorCode::ConnectionFailed
                | AcpErrorCode::ProcessCrashed
                | AcpErrorCode::PromptTimeout
                | AcpErrorCode::RequestCancelled
                | AcpErrorCode::InternalError
        );
        Self::new(code, err.message, retryable)
    }
}

impl From<std::io::Error> for AcpLayerError {
    fn from(err: std::io::Error) -> Self {
        let retryable = matches!(
            err.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::UnexpectedEof
        );
        Self::new(AcpErrorCode::ConnectionFailed, err.to_string(), retryable)
    }
}

pub(crate) fn push_stderr_tail(buf: &std::sync::Arc<std::sync::Mutex<String>>, text: &str) {
    if text.is_empty() {
        return;
    }
    let Ok(mut guard) = buf.lock() else {
        return;
    };
    guard.push_str(text);
    if guard.len() > STDERR_TAIL_LIMIT {
        let keep_from = guard.len().saturating_sub(STDERR_TAIL_LIMIT);
        let next = guard
            .char_indices()
            .find_map(|(idx, _)| (idx >= keep_from).then_some(idx))
            .unwrap_or(keep_from);
        guard.drain(..next);
    }
}

pub(crate) fn stderr_tail(buf: &std::sync::Arc<std::sync::Mutex<String>>) -> String {
    buf.lock().map(|s| s.clone()).unwrap_or_default()
}

fn looks_auth_related(message: &str) -> bool {
    let l = message.to_ascii_lowercase();
    l.contains("auth")
        || l.contains("login")
        || l.contains("credential")
        || l.contains("api key")
        || l.contains("unauthorized")
}

fn looks_connection_related(message: &str) -> bool {
    let l = message.to_ascii_lowercase();
    l.contains("connection closed")
        || l.contains("server shut down")
        || l.contains("broken pipe")
        || l.contains("epipe")
        || l.contains("eof")
        || l.contains("transport closed")
        || l.contains("connection reset")
}
