use crate::acp::client::JockeyUiClient;
use crate::acp::protocol as acp;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

type PendingRequests = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, acp::Error>>>>>;

const ACP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

struct PendingRequestGuard {
    pending: PendingRequests,
    key: String,
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.key);
        }
    }
}

pub(crate) struct AcpConnection {
    stdin: Rc<AsyncMutex<ChildStdin>>,
    pending: PendingRequests,
}

impl AcpConnection {
    pub(crate) fn new(stdin: ChildStdin) -> Self {
        Self {
            stdin: Rc::new(AsyncMutex::new(stdin)),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn request<Req, Resp>(
        &self,
        method: &str,
        params: Req,
    ) -> Result<Resp, acp::Error>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let id = Value::String(uuid::Uuid::new_v4().to_string());
        let key = id_key(&id);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| internal_error("ACP pending request map poisoned"))?
            .insert(key.clone(), tx);
        let _guard = PendingRequestGuard {
            pending: self.pending.clone(),
            key: key.clone(),
        };

        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write_message(&message).await {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&key);
            }
            return Err(internal_error(error));
        }

        let response = tokio::time::timeout(ACP_REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| internal_error(format!("ACP request timed out for {method}")))?
            .map_err(|_| internal_error(format!("ACP response channel closed for {method}")))??;
        serde_json::from_value(response)
            .map_err(|error| internal_error(format!("invalid ACP response for {method}: {error}")))
    }

    pub(crate) async fn notify<Notif: Serialize>(
        &self,
        method: &str,
        params: Notif,
    ) -> Result<(), acp::Error> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
        .map_err(internal_error)
    }

    pub(crate) async fn initialize(
        &self,
        request: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        self.request("initialize", request).await
    }

    pub(crate) async fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        self.request("session/new", request).await
    }

    pub(crate) async fn load_session(
        &self,
        request: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        self.request("session/load", request).await
    }

    pub(crate) async fn prompt(
        &self,
        request: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        self.request("session/prompt", request).await
    }

    pub(crate) async fn cancel(&self, notification: acp::CancelNotification) {
        let _ = self.notify("session/cancel", notification).await;
    }

    pub(crate) async fn set_session_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        self.request("session/set_mode", request).await
    }

    pub(crate) async fn set_session_config_option(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> Result<acp::SetSessionConfigOptionResponse, acp::Error> {
        self.request("session/set_config_option", request).await
    }

    async fn write_message(&self, message: &Value) -> Result<(), String> {
        let line = serde_json::to_vec(message).map_err(|error| error.to_string())?;
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|error| error.to_string())?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|error| error.to_string())?;
        stdin.flush().await.map_err(|error| error.to_string())
    }
}

pub(crate) async fn run_io(
    stdout: ChildStdout,
    connection: Rc<AcpConnection>,
    client: Rc<JockeyUiClient>,
) -> Result<(), String> {
    let pending = connection.pending.clone();
    let result = run_io_inner(stdout, connection, client).await;
    if let Err(error) = &result {
        fail_pending(&pending, error.clone());
    }
    result
}

async fn run_io_inner(
    stdout: ChildStdout,
    connection: Rc<AcpConnection>,
    client: Rc<JockeyUiClient>,
) -> Result<(), String> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await.map_err(|error| error.to_string())? {
        let messages = parse_frame(&line)?;
        for message in messages {
            let Some(object) = message.as_object() else {
                return Err("invalid ACP message: expected object".to_string());
            };
            if let Some(method) = object.get("method").and_then(Value::as_str) {
                let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
                if method == "session/update" {
                    let notification =
                        serde_json::from_value::<acp::SessionNotification>(params)
                            .map_err(|error| format!("invalid ACP session/update: {error}"))?;
                    if let Err(error) = client.session_notification(notification).await {
                        crate::acp::adapter::acp_log(
                            "session.update.error",
                            json!({ "error": error.to_string() }),
                        );
                    }
                    continue;
                }
                let Some(id) = object.get("id").cloned() else {
                    crate::acp::adapter::acp_log(
                        "acp.notification.unknown",
                        json!({ "method": method }),
                    );
                    continue;
                };
                let method = method.to_string();
                let client = client.clone();
                let connection = connection.clone();
                tokio::task::spawn_local(async move {
                    let response = client.dispatch_request(&method, params).await;
                    let message = match response {
                        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                        Err(error) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": serde_json::to_value(error).unwrap_or_else(|_| json!({
                                "code": -32603,
                                "message": "internal error"
                            }))
                        }),
                    };
                    if let Err(error) = connection.write_message(&message).await {
                        crate::acp::adapter::acp_log(
                            "request.response.error",
                            json!({ "method": method, "error": error }),
                        );
                    }
                });
                continue;
            }
            if let Some(id) = object.get("id") {
                let key = id_key(id);
                let result = if let Some(error) = object.get("error") {
                    Err(parse_error(error))
                } else if let Some(result) = object.get("result") {
                    Ok(result.clone())
                } else {
                    Err(internal_error("ACP response has neither result nor error"))
                };
                if let Ok(mut pending) = connection.pending.lock() {
                    if let Some(tx) = pending.remove(&key) {
                        let _ = tx.send(result);
                    } else {
                        // Response for an unknown/expired request id (e.g. the
                        // requester already timed out): surface for diagnosis.
                        crate::acp::adapter::acp_log("acp.response.orphan", json!({ "id": id }));
                    }
                }
            }
        }
    }

    Err("ACP agent closed stdout".to_string())
}

/// Parse one JSONL frame into its member messages. SDK 2.0 batch frames
/// (a JSON array of JSON-RPC messages) are split so each member is dispatched
/// independently; empty lines are ignored.
fn parse_frame(line: &str) -> Result<Vec<Value>, String> {
    if line.trim().is_empty() {
        return Ok(Vec::new());
    }
    let message: Value = serde_json::from_str(line).map_err(|error| {
        format!(
            "invalid ACP JSONL message: {error}; line={}",
            clip_line(line)
        )
    })?;
    Ok(match message {
        Value::Array(messages) => messages,
        other => vec![other],
    })
}

fn fail_pending(pending: &PendingRequests, message: impl Into<String>) {
    let error = internal_error(message);
    if let Ok(mut pending) = pending.lock() {
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(error.clone()));
        }
    }
}

fn parse_error(value: &Value) -> acp::Error {
    serde_json::from_value(value.clone())
        .unwrap_or_else(|_| internal_error("invalid ACP error response"))
}

fn internal_error(message: impl Into<String>) -> acp::Error {
    acp::Error::new(acp::ErrorCode::InternalError.into(), message)
}

fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| id.to_string())
}

fn clip_line(line: &str) -> String {
    const LIMIT: usize = 512;
    if line.len() <= LIMIT {
        return line.to_string();
    }
    let mut end = LIMIT;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_batch_array_frame() {
        let frame =
            r#"[{"jsonrpc":"2.0","id":1,"method":"a"},{"jsonrpc":"2.0","id":2,"method":"b"}]"#;
        let messages = parse_frame(frame).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["id"], serde_json::json!(1));
        assert_eq!(messages[1]["id"], serde_json::json!(2));
    }

    #[test]
    fn parses_single_message_frame() {
        let messages = parse_frame(r#"{"jsonrpc":"2.0","id":"x","result":{}}"#).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], serde_json::json!("x"));
    }

    #[test]
    fn empty_lines_are_skipped() {
        assert!(parse_frame("   ").unwrap().is_empty());
        assert!(parse_frame("").unwrap().is_empty());
    }

    #[test]
    fn invalid_json_reports_error_with_clipped_line() {
        let long_garbage = "x".repeat(2048);
        let error = parse_frame(&long_garbage).unwrap_err();
        assert!(error.contains("invalid ACP JSONL message"));
        assert!(error.chars().count() < 600);
    }

    #[tokio::test]
    async fn fail_pending_flushes_all_waiters() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        pending.lock().unwrap().insert("1".to_string(), tx1);
        pending.lock().unwrap().insert("2".to_string(), tx2);
        fail_pending(&pending, "agent closed stdout");
        assert!(pending.lock().unwrap().is_empty());
        assert!(matches!(rx1.await, Ok(Err(_))));
        assert!(matches!(rx2.await, Ok(Err(_))));
    }
}
