//! Codex thread administration built on the app-server protocol.
//!
//! Field shapes verified against codex 0.150.1's generated v2 schema:
//! - `thread/list` params: `{limit, cursor, cwd, searchTerm, sortKey,
//!   sortDirection, ...}` → `{data: [Thread], nextCursor}` where `Thread`
//!   carries `{id, name, cwd, preview, updatedAt, createdAt, ...}`.
//! - `thread/fork` params: `{threadId, lastTurnId?, cwd, ...}` → `{thread}`.
//! - `thread/rollback` params: `{threadId, numTurns}` → `{thread}`.
//!
//! Listing/forking spawn a short-lived app-server probe process; rollback
//! must run on the live session slot that owns the thread.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::super::super::adapter::{acp_log, NativeProtocol};
use super::NativeProcess;

const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderThreadSummary {
    pub(crate) provider_session_id: String,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) preview: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) created_at: Option<String>,
}

async fn initialize_probe(process: &mut NativeProcess) -> Result<(), String> {
    let _ = process
        .request(
            NativeProtocol::CodexAppServer,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "jockey",
                    "title": "Jockey",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {}
            }),
            CONTROL_TIMEOUT,
            true,
        )
        .await?;
    process
        .send(json!({ "method": "initialized", "params": {} }))
        .await
}

/// List recent codex threads via a short-lived app-server probe process.
pub(crate) async fn list_codex_threads(
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &str,
    limit: u32,
) -> Result<Vec<ProviderThreadSummary>, String> {
    let mut process = NativeProcess::spawn(binary, args, env, cwd, "codex-thread-list")?;
    let result = list_codex_threads_inner(&mut process, limit).await;
    process.close().await;
    result
}

async fn list_codex_threads_inner(
    process: &mut NativeProcess,
    limit: u32,
) -> Result<Vec<ProviderThreadSummary>, String> {
    initialize_probe(process).await?;
    let (response, _) = process
        .request(
            NativeProtocol::CodexAppServer,
            "thread/list",
            json!({
                "limit": limit,
                "sortKey": "recency",
                "sortDirection": "desc",
            }),
            CONTROL_TIMEOUT,
            true,
        )
        .await?;
    parse_thread_list(&response)
}

fn parse_thread_list(response: &Value) -> Result<Vec<ProviderThreadSummary>, String> {
    let Some(items) = response.get("data").and_then(Value::as_array) else {
        return Err("unexpected thread/list response: missing data[]".to_string());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        out.push(ProviderThreadSummary {
            provider_session_id: id.to_string(),
            title: item
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            cwd: item
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            preview: item
                .get("preview")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            updated_at: item
                .get("updatedAt")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            created_at: item
                .get("createdAt")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    Ok(out)
}

/// Fork a persisted codex thread into a new thread via a probe process and
/// return the new thread id.
pub(crate) async fn fork_codex_thread(
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &str,
    thread_id: &str,
) -> Result<String, String> {
    let mut process = NativeProcess::spawn(binary, args, env, cwd, "codex-thread-fork")?;
    let result = fork_codex_thread_inner(&mut process, thread_id).await;
    process.close().await;
    result
}

async fn fork_codex_thread_inner(
    process: &mut NativeProcess,
    thread_id: &str,
) -> Result<String, String> {
    initialize_probe(process).await?;
    let (response, _) = process
        .request(
            NativeProtocol::CodexAppServer,
            "thread/fork",
            json!({ "threadId": thread_id }),
            CONTROL_TIMEOUT,
            true,
        )
        .await?;
    let new_id = response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    match new_id {
        Some(id) if !id.is_empty() => Ok(id),
        _ => Err("unexpected thread/fork response: missing thread.id".to_string()),
    }
}

/// Roll back the live codex thread owned by `process` by `num_turns`.
pub(super) async fn rollback_codex_thread(
    process: &mut NativeProcess,
    thread_id: &str,
    num_turns: u32,
) -> Result<(), String> {
    let (response, _) = process
        .request(
            NativeProtocol::CodexAppServer,
            "thread/rollback",
            json!({ "threadId": thread_id, "numTurns": num_turns }),
            CONTROL_TIMEOUT,
            true,
        )
        .await?;
    if response.get("thread").is_none() {
        return Err("unexpected thread/rollback response: missing thread".to_string());
    }
    acp_log(
        "native.codex.thread_rollback",
        json!({ "threadId": thread_id, "numTurns": num_turns }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thread_list_response() {
        // Shape verified against codex 0.150.1's generated v2 schema.
        let response = json!({
            "data": [
                {
                    "id": "b7f1c2de-1111-2222-3333-444455556666",
                    "name": "refactor auth module",
                    "cwd": "/repo",
                    "preview": "extract the login flow",
                    "updatedAt": "2026-01-05T10:00:00Z",
                    "createdAt": "2026-01-05T09:00:00Z"
                },
                {
                    "id": "0000",
                    "name": null
                }
            ],
            "nextCursor": null
        });
        let threads = parse_thread_list(&response).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(
            threads[0].provider_session_id,
            "b7f1c2de-1111-2222-3333-444455556666"
        );
        assert_eq!(threads[0].title.as_deref(), Some("refactor auth module"));
        assert_eq!(threads[0].cwd.as_deref(), Some("/repo"));
        assert_eq!(
            threads[0].updated_at.as_deref(),
            Some("2026-01-05T10:00:00Z")
        );
        // Items missing optional fields still produce a summary keyed by id.
        assert_eq!(threads[1].provider_session_id, "0000");
        assert!(threads[1].title.is_none());
    }

    #[test]
    fn rejects_thread_list_without_data() {
        assert!(parse_thread_list(&json!({})).is_err());
    }
}

/// Real smoke test against the locally installed codex CLI. Ignored by
/// default (requires codex on PATH and writable ~/.codex state); run with
/// `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_smoke`.
#[tokio::test]
#[ignore]
async fn real_smoke_codex_thread_list() {
    let launch = crate::acp::adapter::resolve_adapter_launch("codex-cli")
        .expect("codex-cli adapter must resolve for real smoke");
    let threads = list_codex_threads(&launch.binary, &launch.args, &launch.env, ".", 5)
        .await
        .expect("thread/list must succeed against the local app-server");
    assert!(threads.len() <= 5);
    for thread in &threads {
        assert!(!thread.provider_session_id.is_empty());
    }
}
