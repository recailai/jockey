use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::super::adapter::{acp_log, clip};
use super::super::client::JockeyUiClient;
use super::super::error::{push_stderr_tail, stderr_tail, AcpLayerError};
use super::super::metrics::{
    record_init_latency, record_session_start_latency, record_spawn_latency,
};
use super::super::runtime_state::{
    remember_runtime_config_options, remember_runtime_models, remember_runtime_modes,
};
use super::super::worker::{
    register_child_pid, ConfigStateCell, DeltaSlot, LiveConnection, ModeStateCell,
};

pub(super) const INIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(super) const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(super) static LIVE_CONNECTION_SEQ: AtomicU64 = AtomicU64::new(1);

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

pub(crate) async fn cold_start(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
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
        .kill_on_drop(true)
        .process_group(0);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }

    let spawn_started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    record_spawn_latency(runtime_key, spawn_started.elapsed().as_millis());
    if let Some(pid) = child.id() {
        register_child_pid(pid);
    }
    let stdin = child.stdin.take().ok_or("stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;
    let stderr_buf = Arc::new(Mutex::new(String::new()));

    if let Some(stderr) = child.stderr.take() {
        let bin = binary.to_string();
        let stderr_buf = stderr_buf.clone();
        tokio::task::spawn_local(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut r = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.trim().is_empty() => {
                        push_stderr_tail(&stderr_buf, &line);
                        acp_log(
                            "stderr",
                            json!({ "binary": bin, "line": clip(line.trim(), 360) }),
                        );
                    }
                    _ => {}
                }
            }
        });
    }

    acp_log("spawn.ok", json!({ "binary": binary, "pid": child.id() }));

    let expected_session_id = Arc::new(Mutex::new(None));
    let mode_state: ModeStateCell = Rc::new(RefCell::new(None));
    let config_state: ConfigStateCell = Rc::new(RefCell::new(Vec::new()));
    let terminals: super::super::client::TerminalMap =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let (conn, io_future) = acp::ClientSideConnection::new(
        JockeyUiClient {
            delta_slot: delta_slot.clone(),
            auto_approve,
            runtime_key: runtime_key.to_string(),
            role_name: role_name.to_string(),
            app_session_id: app_session_id.to_string(),
            expected_session_id: expected_session_id.clone(),
            mode_state: mode_state.clone(),
            config_state: config_state.clone(),
            terminals,
        },
        stdin.compat_write(),
        stdout.compat(),
        |fut| {
            tokio::task::spawn_local(fut);
        },
    );

    let (health_tx, mut health_rx) = tokio::sync::watch::channel(true);
    let io_handle = tokio::task::spawn_local(async move {
        if let Err(e) = io_future.await {
            acp_log("io_task.error", json!({ "error": e.to_string() }));
        }
        let _ = health_tx.send(false);
    });

    let t = Instant::now();
    let init_resp = {
        let init_fut = conn.initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1)
                .client_info(acp::Implementation::new("jockey", "0.1.0").title("Jockey"))
                .client_capabilities(
                    acp::ClientCapabilities::new()
                        .fs(acp::FileSystemCapabilities::new()
                            .read_text_file(true)
                            .write_text_file(true))
                        .terminal(true)
                        .meta(acp::Meta::from_iter([(
                            "terminal_output".to_string(),
                            true.into(),
                        )])),
                ),
        );
        tokio::pin!(init_fut);
        tokio::select! {
            res = &mut init_fut => res.map_err(|e| AcpLayerError::from(e).into_message())?,
            _ = tokio::time::sleep(INIT_TIMEOUT) => {
                return Err(AcpLayerError::timeout("initialize", INIT_TIMEOUT.as_secs()).into_message());
            }
            status = child.wait() => {
                let detail = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                return Err(AcpLayerError::process_crashed(&format!("initialize ({detail})"), &stderr_tail(&stderr_buf)).into_message());
            }
            changed = health_rx.changed() => {
                if changed.is_err() || !*health_rx.borrow() {
                    return Err(AcpLayerError::connection_closed(format!("connection closed during initialize: {}", stderr_tail(&stderr_buf).trim())).into_message());
                }
                return Err(AcpLayerError::connection_closed("connection health changed during initialize").into_message());
            }
        }
    };
    acp_log(
        "stage.ok",
        json!({ "stage": "initialize", "latencyMs": t.elapsed().as_millis() }),
    );
    record_init_latency(runtime_key, t.elapsed().as_millis());

    let t = Instant::now();
    let supports_load = init_resp.agent_capabilities.load_session;

    struct SessionStartResult {
        session_id: acp::SessionId,
        config_options: Option<Vec<acp::SessionConfigOption>>,
        modes: Option<acp::SessionModeState>,
        resumed: bool,
    }

    async fn do_new_session(
        conn: &acp::ClientSideConnection,
        abs_cwd: &str,
        mcp_servers: Vec<acp::McpServer>,
        stderr_buf: Arc<Mutex<String>>,
    ) -> Result<SessionStartResult, String> {
        let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd));
        if !mcp_servers.is_empty() {
            req = req.mcp_servers(mcp_servers);
        }
        let resp = tokio::time::timeout(SESSION_TIMEOUT, conn.new_session(req))
            .await
            .map_err(|_| {
                AcpLayerError::timeout("new_session", SESSION_TIMEOUT.as_secs()).into_message()
            })?
            .map_err(|e| {
                let mut msg = AcpLayerError::from(e).into_message();
                let tail = stderr_tail(&stderr_buf);
                if !tail.trim().is_empty() {
                    msg.push_str(": ");
                    msg.push_str(tail.trim());
                }
                msg
            })?;
        Ok(SessionStartResult {
            session_id: resp.session_id,
            config_options: resp.config_options,
            modes: resp.modes,
            resumed: false,
        })
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
            match tokio::time::timeout(SESSION_TIMEOUT, conn.load_session(load_req))
                .await
                .map_err(|_| {
                    acp::Error::new(
                        acp::ErrorCode::InternalError.into(),
                        format!(
                            "timeout: load_session exceeded {}s",
                            SESSION_TIMEOUT.as_secs()
                        ),
                    )
                })
                .and_then(|r| r)
            {
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
                        json!({ "runtime": runtime_key, "error": AcpLayerError::from(e).into_message(), "stderr": clip(&stderr_tail(&stderr_buf), 360) }),
                    );
                    do_new_session(&conn, abs_cwd, mcp_servers, stderr_buf.clone()).await?
                }
            }
        } else {
            do_new_session(&conn, abs_cwd, mcp_servers, stderr_buf.clone()).await?
        }
    } else {
        do_new_session(&conn, abs_cwd, mcp_servers, stderr_buf.clone()).await?
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
    if let Some(ref opts) = start_result.config_options {
        *config_state.borrow_mut() = opts.clone();
    }
    let session_id = start_result.session_id;
    if let Ok(mut guard) = expected_session_id.lock() {
        *guard = Some(session_id.clone());
    }

    if let Some(ref modes) = start_result.modes {
        *mode_state.borrow_mut() = Some(modes.clone());
    }
    let available_modes_ids: Vec<String> = start_result
        .modes
        .as_ref()
        .map(|m| {
            m.available_modes
                .iter()
                .map(|mode| mode.id.to_string())
                .collect()
        })
        .unwrap_or_default();
    remember_runtime_modes(runtime_key, available_modes_ids);

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
    record_session_start_latency(runtime_key, t.elapsed().as_millis());

    Ok(LiveConnection {
        instance_id: LIVE_CONNECTION_SEQ.fetch_add(1, Ordering::Relaxed),
        conn: Rc::new(conn),
        session_id,
        cwd: abs_cwd.to_string(),
        delta_slot,
        mode_state,
        config_state,
        child_pid: child.id(),
        last_active: Instant::now(),
        _child: child,
        _io_task: io_handle,
        health_rx,
    })
}
