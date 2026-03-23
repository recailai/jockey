use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::adapter::{acp_log, build_stdio_adapter, clip, friendly_error_message};
use super::client::UnionAiClient;
use super::worker::{
    remember_runtime_available_commands, remember_runtime_config_options, remember_runtime_models,
    remember_runtime_modes, worker_tx, AcpEvent, AcpPromptResult, DeltaSlot, LiveConnection,
    RuntimeKind, WorkerMsg,
};
use crate::db::app_session::{load_app_session_role_cli_id, save_app_session_role_cli_id};
use crate::types::AppState;

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
        let is_model_category = matches!(
            option.category,
            Some(acp::SessionConfigOptionCategory::Model)
        );
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

pub(super) fn extract_mode_ids(modes: &[Value]) -> Vec<String> {
    let mut out = HashSet::new();
    for mode in modes {
        if let Some(id) = mode.get("id").and_then(|v| v.as_str()) {
            out.insert(id.to_string());
            continue;
        }
        if let Some(id) = mode.as_str() {
            out.insert(id.to_string());
        }
    }
    let mut items = out.into_iter().collect::<Vec<_>>();
    items.sort_unstable();
    items
}

pub(super) async fn cold_start(
    runtime_key: &'static str,
    binary: &str,
    args: &[String],
    env_pairs: &[(String, String)],
    abs_cwd: &str,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    resume_session_id: Option<String>,
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
    let init_resp = conn
        .initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1)
                .client_info(acp::Implementation::new("unionai", "0.1.0").title("UnionAI"))
                .client_capabilities(
                    acp::ClientCapabilities::new()
                        .fs(acp::FileSystemCapabilities::new()
                            .read_text_file(true)
                            .write_text_file(true))
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
    let supports_load = init_resp.agent_capabilities.load_session;

    struct SessionStartResult {
        session_id: acp::SessionId,
        config_options: Option<Vec<acp::SessionConfigOption>>,
        modes: Option<acp::SessionModeState>,
        resumed: bool,
    }

    let start_result: SessionStartResult = if supports_load {
        if let Some(sid) = resume_session_id {
            let mut load_req = acp::LoadSessionRequest::new(
                acp::SessionId::from(sid.clone()),
                std::path::PathBuf::from(abs_cwd),
            );
            if !mcp_servers.is_empty() {
                load_req = load_req.mcp_servers(mcp_servers.clone());
            }
            match conn.load_session(load_req).await {
                Ok(resp) => {
                    acp_log(
                        "session.load.ok",
                        json!({ "runtime": runtime_key, "sessionId": &sid }),
                    );
                    SessionStartResult {
                        session_id: acp::SessionId::from(sid),
                        config_options: resp.config_options,
                        modes: resp.modes,
                        resumed: true,
                    }
                }
                Err(e) => {
                    acp_log(
                        "session.load.fallback",
                        json!({ "runtime": runtime_key, "error": e.to_string() }),
                    );
                    let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd));
                    if !mcp_servers.is_empty() {
                        req = req.mcp_servers(mcp_servers);
                    }
                    let resp = conn.new_session(req).await.map_err(|e| e.to_string())?;
                    SessionStartResult {
                        session_id: resp.session_id,
                        config_options: resp.config_options,
                        modes: resp.modes,
                        resumed: false,
                    }
                }
            }
        } else {
            let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd));
            if !mcp_servers.is_empty() {
                req = req.mcp_servers(mcp_servers);
            }
            let resp = conn.new_session(req).await.map_err(|e| e.to_string())?;
            SessionStartResult {
                session_id: resp.session_id,
                config_options: resp.config_options,
                modes: resp.modes,
                resumed: false,
            }
        }
    } else {
        let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd));
        if !mcp_servers.is_empty() {
            req = req.mcp_servers(mcp_servers);
        }
        let resp = conn.new_session(req).await.map_err(|e| e.to_string())?;
        SessionStartResult {
            session_id: resp.session_id,
            config_options: resp.config_options,
            modes: resp.modes,
            resumed: false,
        }
    };

    let discovered_models =
        extract_models_from_config_options(start_result.config_options.as_ref());
    remember_runtime_models(runtime_key, discovered_models.clone());
    if let Some(ref opts) = start_result.config_options {
        let serialized: Vec<Value> = opts
            .iter()
            .filter_map(|o| serde_json::to_value(o).ok())
            .collect();
        acp_log(
            "config_options.discovered",
            json!({
                "runtime": runtime_key,
                "count": serialized.len(),
                "ids": serialized.iter().filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()
            }),
        );
        remember_runtime_config_options(runtime_key, serialized);
    } else {
        acp_log("config_options.none", json!({ "runtime": runtime_key }));
    }
    let session_id = start_result.session_id;

    let available_modes = start_result
        .modes
        .as_ref()
        .map(|m| {
            serde_json::to_value(m)
                .ok()
                .and_then(|v| v.get("modes").cloned())
                .unwrap_or(json!([]))
        })
        .unwrap_or(json!([]));
    let available_modes = match available_modes {
        Value::Array(a) => a,
        _ => vec![],
    };
    remember_runtime_modes(runtime_key, extract_mode_ids(&available_modes));
    let current_mode = start_result.modes.as_ref().and_then(|m| {
        serde_json::to_value(m).ok().and_then(|v| {
            v.get("current")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        })
    });

    acp_log(
        "stage.ok",
        json!({
            "stage": if start_result.resumed { "session/load" } else { "session/new" },
            "latencyMs": t.elapsed().as_millis(),
            "sessionId": session_id.to_string(),
            "runtime": runtime_key,
            "discoveredModelCount": discovered_models.len(),
            "resumed": start_result.resumed
        }),
    );

    Ok(LiveConnection {
        conn: Rc::new(conn),
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
    state: Option<(&AppState, &str)>,
    app_session_id: &str,
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

    let resume_session_id = state.as_ref().and_then(|(s, app_sid)| {
        load_app_session_role_cli_id(s, app_sid, adapter.runtime_key, role_name)
    });
    let app_session_scope = if app_session_id.trim().is_empty() {
        None
    } else {
        Some(app_session_id.to_string())
    };

    let (delta_tx, mut delta_rx) = mpsc::unbounded_channel::<AcpEvent>();
    let (result_tx, mut result_rx) = oneshot::channel();

    let _ = worker_tx().send(WorkerMsg::Execute {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary.clone(),
        args: adapter.args.clone(),
        env: adapter.env.clone(),
        role_name: role_name.to_string(),
        app_session_id: app_session_scope,
        prompt: prompt.to_string(),
        context: context.to_vec(),
        cwd: cwd.to_string(),
        delta_tx,
        result_tx,
        auto_approve,
        mcp_servers,
        role_mode,
        role_config_options,
        resume_session_id,
    });

    let app = app.clone();
    let role_owned = role_name.to_string();
    let app_session_id_owned = app_session_id.to_string();
    let mut full_output = String::new();
    let mut delta_count = 0usize;

    acp_log("execute.stream.listening", json!({
        "runtime": adapter.runtime_key,
        "role": role_owned,
        "prompt": clip(prompt, 80),
    }));

    let heartbeat = tokio::time::Instant::now();
    let mut heartbeat_count = 0u32;
    let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(5));
    heartbeat_interval.tick().await; // consume the immediate first tick
    let result = loop {
        tokio::select! {
            _ = heartbeat_interval.tick() => {
                heartbeat_count += 1;
                acp_log("execute.heartbeat", json!({
                    "runtime": adapter.runtime_key,
                    "role": role_owned,
                    "elapsedSec": heartbeat.elapsed().as_secs(),
                    "deltaCount": delta_count,
                    "beat": heartbeat_count,
                }));
                continue;
            }
            evt = delta_rx.recv() => {
                match evt {
                    Some(AcpEvent::TextDelta { ref text }) => {
                        full_output.push_str(text);
                        delta_count += 1;
                        acp_log("delta.text", json!({
                            "runtime": adapter.runtime_key,
                            "role": role_owned,
                            "deltaIndex": delta_count,
                            "chunkLen": text.len(),
                            "preview": clip(text, 60),
                        }));
                        let _ = app.emit("acp/delta", json!({
                            "role": role_owned,
                            "runtimeKind": adapter.runtime_key,
                            "appSessionId": app_session_id_owned,
                            "delta": text
                        }));
                    }
                    Some(AcpEvent::ConfigUpdate { ref options }) => {
                        delta_count += 1;
                        remember_runtime_config_options(adapter.runtime_key, options.clone());
                        let _ = app.emit("acp/stream", json!({
                            "role": role_owned,
                            "runtimeKind": adapter.runtime_key,
                            "appSessionId": app_session_id_owned,
                            "event": serde_json::to_value(&AcpEvent::ConfigUpdate { options: options.clone() }).unwrap_or(json!({}))
                        }));
                    }
                    Some(AcpEvent::AvailableCommands { ref commands }) => {
                        delta_count += 1;
                        acp_log("commands.discovered", json!({
                            "runtime": adapter.runtime_key,
                            "role": role_owned,
                            "count": commands.len(),
                            "names": commands.iter().filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()
                        }));
                        remember_runtime_available_commands(adapter.runtime_key, &role_owned, commands.clone());
                        let _ = app.emit("acp/stream", json!({
                            "role": role_owned,
                            "runtimeKind": adapter.runtime_key,
                            "appSessionId": app_session_id_owned,
                            "event": serde_json::to_value(&AcpEvent::AvailableCommands { commands: commands.clone() }).unwrap_or(json!({}))
                        }));
                    }
                    Some(ref other) => {
                        delta_count += 1;
                        let event_json = serde_json::to_value(&other).unwrap_or(json!({}));
                        acp_log("delta.event", json!({
                            "runtime": adapter.runtime_key,
                            "role": role_owned,
                            "deltaIndex": delta_count,
                            "kind": event_json.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            "preview": clip(&event_json.to_string(), 120),
                        }));
                        let _ = app.emit("acp/stream", json!({
                            "role": role_owned,
                            "runtimeKind": adapter.runtime_key,
                            "appSessionId": app_session_id_owned,
                            "event": event_json
                        }));
                    }
                    None => {}
                }
            }
            res = &mut result_rx => {
                while let Ok(evt) = delta_rx.try_recv() {
                    match evt {
                        AcpEvent::TextDelta { ref text } => {
                            full_output.push_str(text);
                            delta_count += 1;
                            let _ = app.emit("acp/delta", json!({ "role": role_owned, "runtimeKind": adapter.runtime_key, "appSessionId": app_session_id_owned, "delta": text }));
                        }
                        other => {
                            delta_count += 1;
                            let _ = app.emit("acp/stream", json!({
                                "role": role_owned,
                                "runtimeKind": adapter.runtime_key,
                                "appSessionId": app_session_id_owned,
                                "event": serde_json::to_value(&other).unwrap_or(json!({}))
                            }));
                        }
                    }
                }
                let r = res.unwrap_or_else(|_| Err("worker disconnected".to_string()));
                acp_log("execute.result", json!({
                    "runtime": adapter.runtime_key,
                    "role": role_owned,
                    "ok": r.is_ok(),
                    "deltaCount": delta_count,
                    "outputSize": full_output.len(),
                    "outputPreview": clip(&full_output, 120),
                }));
                break r;
            }
        }
    };

    match result {
        Ok((_output, session_id)) => {
            if let Some((s, app_sid)) = state {
                if !session_id.is_empty() {
                    let _ = save_app_session_role_cli_id(
                        s,
                        app_sid,
                        adapter.runtime_key,
                        role_name,
                        &session_id,
                    );
                }
            }

            let output = if full_output.is_empty() {
                if prompt.starts_with('/') {
                    format!(
                        "Command sent to {} (role: {}). Agent processed it with {} event(s) but returned no text output.",
                        adapter.runtime_key, role_name, delta_count
                    )
                } else {
                    format!(
                        "{} completed for role {} with no text output ({} event(s)).",
                        adapter.runtime_key, role_name, delta_count
                    )
                }
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

async fn send_prewarm(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    resume_session_id: Option<String>,
    app_session_id: Option<&str>,
) -> Option<oneshot::Receiver<(Vec<serde_json::Value>, String)>> {
    let adapter = match build_stdio_adapter(runtime_kind) {
        Ok(Some(a)) => a,
        _ => return None,
    };
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::Prewarm {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary,
        args: adapter.args,
        env: adapter.env,
        role_name: role_name.to_string(),
        app_session_id: app_session_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string()),
        cwd: cwd.to_string(),
        auto_approve: true,
        mcp_servers: vec![],
        role_mode: None,
        role_config_options: vec![],
        result_tx: Some(tx),
        resume_session_id,
    });
    Some(rx)
}

pub async fn prewarm_role(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    state: Option<(&AppState, &str)>,
) {
    let runtime_key = normalize_runtime_key(runtime_kind).unwrap_or(runtime_kind);
    let resume_session_id = state
        .as_ref()
        .and_then(|(s, sid)| load_app_session_role_cli_id(s, sid, runtime_key, role_name));
    let app_session_id = state.as_ref().map(|(_, sid)| *sid);
    let Some(rx) = send_prewarm(
        runtime_kind,
        role_name,
        cwd,
        resume_session_id,
        app_session_id,
    )
    .await
    else {
        return;
    };
    if let Some((s, app_sid)) = state {
        if let Ok((_opts, sid)) = rx.await {
            if !sid.is_empty() {
                let _ = save_app_session_role_cli_id(s, app_sid, runtime_key, role_name, &sid);
            }
        }
    }
}

pub async fn prewarm_role_for_config(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    state: Option<(&AppState, &str)>,
) -> Vec<serde_json::Value> {
    let runtime_key = normalize_runtime_key(runtime_kind).unwrap_or(runtime_kind);
    let resume_session_id = state
        .as_ref()
        .and_then(|(s, sid)| load_app_session_role_cli_id(s, sid, runtime_key, role_name));
    let app_session_id = state.as_ref().map(|(_, sid)| *sid);
    let Some(rx) = send_prewarm(
        runtime_kind,
        role_name,
        cwd,
        resume_session_id,
        app_session_id,
    )
    .await
    else {
        return vec![];
    };
    let (opts, sid) = rx.await.unwrap_or_default();
    if let Some((s, app_sid)) = state {
        if !sid.is_empty() {
            let _ = save_app_session_role_cli_id(s, app_sid, runtime_key, role_name, &sid);
        }
    }
    opts
}

pub async fn prewarm(runtime_kind: &str, cwd: &str) {
    prewarm_role(runtime_kind, "UnionAIAssistant", cwd, None).await;
}

pub async fn prewarm_role_with_session_id(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    resume_session_id: Option<String>,
    state: &AppState,
    app_session_id: &str,
) {
    let runtime_key = normalize_runtime_key(runtime_kind).unwrap_or(runtime_kind);
    let Some(rx) = send_prewarm(
        runtime_kind,
        role_name,
        cwd,
        resume_session_id,
        Some(app_session_id),
    )
    .await
    else {
        return;
    };
    if let Ok((_opts, sid)) = rx.await {
        if !sid.is_empty() {
            let _ =
                save_app_session_role_cli_id(state, app_session_id, runtime_key, role_name, &sid);
        }
    }
}

fn normalize_runtime_key(runtime_kind: &str) -> Option<&'static str> {
    RuntimeKind::from_str(runtime_kind).map(|k| k.runtime_key())
}

pub async fn cancel_session(runtime_kind: &str, role_name: &str, app_session_id: Option<&str>) {
    let Some(runtime_key) = normalize_runtime_key(runtime_kind) else {
        return;
    };
    let _ = worker_tx().send(WorkerMsg::Cancel {
        runtime_key,
        role_name: role_name.to_string(),
        app_session_id: app_session_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string()),
    });
}

pub async fn set_mode(
    runtime_kind: &str,
    role_name: &str,
    mode_id: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetMode {
        runtime_key,
        role_name: role_name.to_string(),
        app_session_id: app_session_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string()),
        mode_id: mode_id.to_string(),
        result_tx: tx,
    });
    rx.await.map_err(|_| "worker disconnected".to_string())?
}

pub async fn set_config_option(
    runtime_kind: &str,
    role_name: &str,
    key: &str,
    value: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetConfigOption {
        runtime_key,
        role_name: role_name.to_string(),
        app_session_id: app_session_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string()),
        config_id: key.to_string(),
        value: value.to_string(),
        result_tx: tx,
    });
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
        meta: json!({ "mode": "mock", "agentKind": RuntimeKind::Mock, "runtimeKey": "mock" }),
    }
}
