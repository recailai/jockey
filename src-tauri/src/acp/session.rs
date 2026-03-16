use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::adapter::{acp_log, build_stdio_adapter, clip, friendly_error_message};
use super::client::UnionAiClient;
use super::worker::{
    AcpEvent, AcpPromptResult, AgentKind, DeltaSlot, LiveConnection, WorkerMsg,
    remember_runtime_models, worker_tx,
};

pub(super) fn collect_models_from_select_options(
    options: &acp::SessionConfigSelectOptions,
    out: &mut HashSet<String>,
) {
    match options {
        acp::SessionConfigSelectOptions::Ungrouped(items) => {
            for item in items {
                out.insert(item.value.to_string());
            }
        }
        acp::SessionConfigSelectOptions::Grouped(groups) => {
            for group in groups {
                for item in &group.options {
                    out.insert(item.value.to_string());
                }
            }
        }
        _ => {}
    }
}

pub(super) fn extract_models_from_config_options(
    options: Option<&Vec<acp::SessionConfigOption>>,
) -> Vec<String> {
    let Some(options) = options else {
        return Vec::new();
    };
    let mut out = HashSet::new();
    for option in options {
        let is_model_category = matches!(option.category, Some(acp::SessionConfigOptionCategory::Model));
        let is_model_id = option.id.to_string().eq_ignore_ascii_case("model");
        if !(is_model_category || is_model_id) {
            continue;
        }
        if let acp::SessionConfigKind::Select(select) = &option.kind {
            out.insert(select.current_value.to_string());
            collect_models_from_select_options(&select.options, &mut out);
        }
    }
    let mut models = out.into_iter().collect::<Vec<_>>();
    models.sort_unstable();
    models
}

pub(super) async fn cold_start(
    runtime_key: &'static str,
    binary: &str,
    args: &[String],
    env_pairs: &[(String, String)],
    abs_cwd: &str,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
) -> Result<LiveConnection, String> {
    let delta_slot: DeltaSlot = Arc::new(Mutex::new(None));
    acp_log("spawn.start", json!({ "binary": binary, "cwd": abs_cwd }));

    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args)
        .current_dir(abs_cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().ok_or("stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;

    if let Some(stderr) = child.stderr.take() {
        let bin = binary.to_string();
        tokio::task::spawn_local(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut r = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.trim().is_empty() => acp_log(
                        "stderr",
                        json!({ "binary": bin, "line": clip(line.trim(), 360) }),
                    ),
                    _ => {}
                }
            }
        });
    }

    acp_log("spawn.ok", json!({ "binary": binary, "pid": child.id() }));

    let (conn, io_future) = acp::ClientSideConnection::new(
        UnionAiClient {
            delta_slot: delta_slot.clone(),
            auto_approve,
        },
        stdin.compat_write(),
        stdout.compat(),
        |fut| {
            tokio::task::spawn_local(fut);
        },
    );

    let io_handle = tokio::task::spawn_local(async move {
        if let Err(e) = io_future.await {
            acp_log("io_task.error", json!({ "error": e.to_string() }));
        }
    });

    let t = Instant::now();
    conn.initialize(
        acp::InitializeRequest::new(acp::ProtocolVersion::V1)
            .client_info(acp::Implementation::new("unionai", "0.1.0").title("UnionAI"))
            .client_capabilities(
                acp::ClientCapabilities::new()
                    .fs(acp::FileSystemCapabilities::new().read_text_file(true).write_text_file(true))
                    .terminal(true),
            ),
    )
    .await
    .map_err(|e| e.to_string())?;
    acp_log(
        "stage.ok",
        json!({ "stage": "initialize", "latencyMs": t.elapsed().as_millis() }),
    );

    let t = Instant::now();
    let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd));
    if !mcp_servers.is_empty() {
        req = req.mcp_servers(mcp_servers);
    }
    let resp = conn
        .new_session(req)
        .await
        .map_err(|e| e.to_string())?;
    let discovered_models = extract_models_from_config_options(resp.config_options.as_ref());
    remember_runtime_models(runtime_key, discovered_models.clone());
    let session_id = resp.session_id;

    let available_modes = resp.modes.as_ref().map(|m| {
        serde_json::to_value(m).ok().and_then(|v| v.get("modes").cloned()).unwrap_or(json!([]))
    }).unwrap_or(json!([]));
    let available_modes = match available_modes {
        Value::Array(a) => a,
        _ => vec![],
    };
    let current_mode = resp.modes.as_ref().and_then(|m| {
        serde_json::to_value(m).ok().and_then(|v| v.get("current").and_then(|c| c.as_str()).map(|s| s.to_string()))
    });

    acp_log(
        "stage.ok",
        json!({
            "stage": "session/new",
            "latencyMs": t.elapsed().as_millis(),
            "sessionId": session_id.to_string(),
            "runtime": runtime_key,
            "discoveredModelCount": discovered_models.len()
        }),
    );

    Ok(LiveConnection {
        conn,
        session_id,
        cwd: abs_cwd.to_string(),
        delta_slot,
        available_modes,
        current_mode,
        _child: child,
        _io_task: io_handle,
    })
}

pub async fn execute_runtime(
    runtime_kind: &str,
    role_name: &str,
    prompt: &str,
    context: &[(String, String)],
    cwd: &str,
    app: &tauri::AppHandle,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
) -> AcpPromptResult {
    let normalized = runtime_kind.trim().to_ascii_lowercase();

    if normalized.is_empty() || normalized == "mock" {
        return mock_execute(role_name, prompt, context);
    }

    let adapter = match build_stdio_adapter(&normalized) {
        Ok(Some(a)) => a,
        Ok(None) => {
            return AcpPromptResult {
                output: format!("unsupported runtime kind: {}", normalized),
                deltas: vec![],
                meta: json!({ "mode": "unsupported-runtime", "runtime": normalized }),
            }
        }
        Err(e) => {
            return AcpPromptResult {
                output: friendly_error_message(&normalized, &e),
                deltas: vec![],
                meta: json!({ "mode": "adapter-unavailable", "runtime": normalized, "error": e }),
            }
        }
    };

    let agent_kind = adapter.kind;
    let started = Instant::now();
    acp_log(
        "execute.start",
        json!({
            "runtime": adapter.runtime_key,
            "role": role_name,
            "promptSize": prompt.len(),
            "cwd": cwd
        }),
    );

    let (delta_tx, mut delta_rx) = mpsc::unbounded_channel::<AcpEvent>();
    let (result_tx, mut result_rx) = oneshot::channel();

    let _ = worker_tx().send(WorkerMsg::Execute {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary.clone(),
        args: adapter.args.clone(),
        env: adapter.env.clone(),
        role_name: role_name.to_string(),
        prompt: prompt.to_string(),
        context: context.to_vec(),
        cwd: cwd.to_string(),
        delta_tx,
        result_tx,
        auto_approve,
        mcp_servers,
        role_mode,
        role_config_options,
    });

    let app = app.clone();
    let role_owned = role_name.to_string();
    let mut full_output = String::new();
    let mut delta_count = 0usize;
    let mut buffer = String::new();
    let mut last_emit = Instant::now();

    let result = loop {
        tokio::select! {
            evt = delta_rx.recv() => {
                match evt {
                    Some(AcpEvent::TextDelta { ref text }) => {
                        full_output.push_str(text);
                        delta_count += 1;
                        buffer.push_str(text);
                        let should_emit = buffer.len() >= 4
                            || text.contains('\n')
                            || last_emit.elapsed() >= std::time::Duration::from_millis(15);
                        if should_emit {
                            let _ = app.emit("acp/delta", json!({
                                "role": role_owned,
                                "delta": buffer.clone()
                            }));
                            buffer.clear();
                            last_emit = Instant::now();
                        }
                    }
                    Some(other) => {
                        delta_count += 1;
                        let _ = app.emit("acp/stream", json!({
                            "role": role_owned,
                            "event": serde_json::to_value(&other).unwrap_or(json!({}))
                        }));
                    }
                    None => {}
                }
            }
            res = &mut result_rx => {
                if !buffer.is_empty() {
                    let _ = app.emit("acp/delta", json!({ "role": role_owned, "delta": buffer.clone() }));
                }
                while let Ok(evt) = delta_rx.try_recv() {
                    match evt {
                        AcpEvent::TextDelta { ref text } => {
                            full_output.push_str(text);
                            delta_count += 1;
                            let _ = app.emit("acp/delta", json!({ "role": role_owned, "delta": text }));
                        }
                        other => {
                            delta_count += 1;
                            let _ = app.emit("acp/stream", json!({
                                "role": role_owned,
                                "event": serde_json::to_value(&other).unwrap_or(json!({}))
                            }));
                        }
                    }
                }
                break res.unwrap_or_else(|_| Err("worker disconnected".to_string()));
            }
        }
    };

    match result {
        Ok((_output, session_id)) => {
            let output = if full_output.is_empty() {
                format!(
                    "{} ACP completed for role {} with no text payload.",
                    adapter.runtime_key, role_name
                )
            } else {
                full_output
            };

            acp_log(
                "execute.ok",
                json!({
                    "runtime": adapter.runtime_key,
                    "role": role_name,
                    "latencyMs": started.elapsed().as_millis(),
                    "deltaCount": delta_count,
                    "outputSize": output.len()
                }),
            );

            AcpPromptResult {
                output,
                deltas: vec![],
                meta: json!({
                    "mode": "live",
                    "agentKind": agent_kind,
                    "runtimeKey": adapter.runtime_key,
                    "sessionId": session_id
                }),
            }
        }
        Err(e) => {
            let friendly = friendly_error_message(adapter.runtime_key, &e);
            AcpPromptResult {
                output: friendly.clone(),
                deltas: vec![],
                meta: json!({ "mode": "acp-error", "runtime": adapter.runtime_key, "error": e, "friendlyMessage": friendly }),
            }
        }
    }
}

pub async fn prewarm_role(runtime_kind: &str, role_name: &str, cwd: &str) {
    let adapter = match build_stdio_adapter(runtime_kind) {
        Ok(Some(a)) => a,
        _ => return,
    };
    let _ = worker_tx().send(WorkerMsg::Prewarm {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary,
        args: adapter.args,
        env: adapter.env,
        role_name: role_name.to_string(),
        cwd: cwd.to_string(),
        auto_approve: true,
        mcp_servers: vec![],
        role_mode: None,
        role_config_options: vec![],
    });
}

pub async fn prewarm(runtime_kind: &str, cwd: &str) {
    prewarm_role(runtime_kind, "UnionAIAssistant", cwd).await;
}

fn normalize_runtime_key(runtime_kind: &str) -> Option<&'static str> {
    let normalized = runtime_kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "claude" | "claude-code" | "claude-acp" => Some("claude-code"),
        "gemini" | "gemini-cli" => Some("gemini-cli"),
        "codex" | "codex-cli" | "codex-acp" => Some("codex-cli"),
        _ => None,
    }
}

pub async fn cancel_session(runtime_kind: &str, role_name: &str) {
    let Some(runtime_key) = normalize_runtime_key(runtime_kind) else { return };
    let _ = worker_tx().send(WorkerMsg::Cancel { runtime_key, role_name: role_name.to_string() });
}

pub async fn set_mode(runtime_kind: &str, role_name: &str, mode_id: &str) -> Result<(), String> {
    let runtime_key = normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetMode { runtime_key, role_name: role_name.to_string(), mode_id: mode_id.to_string(), result_tx: tx });
    rx.await.map_err(|_| "worker disconnected".to_string())?
}

pub async fn set_config_option(runtime_kind: &str, role_name: &str, key: &str, value: &str) -> Result<(), String> {
    let runtime_key = normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetConfigOption { runtime_key, role_name: role_name.to_string(), config_id: key.to_string(), value: value.to_string(), result_tx: tx });
    rx.await.map_err(|_| "worker disconnected".to_string())?
}

fn mock_execute(role: &str, prompt: &str, ctx: &[(String, String)]) -> AcpPromptResult {
    let snap = ctx
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let out = format!("{role} prompt: {prompt}. Context: {snap}.");
    AcpPromptResult {
        output: out.clone(),
        deltas: out
            .as_bytes()
            .chunks(28)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect(),
        meta: json!({ "mode": "mock", "agentKind": AgentKind::Mock, "runtimeKey": "mock" }),
    }
}
