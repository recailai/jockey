use crate::acp::protocol as acp;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::super::adapter::{acp_log, clip};
use super::super::client::JockeyUiClient;
use super::super::error::{push_stderr_tail, stderr_tail, AcpLayerError};
use super::super::metrics::{
    record_init_latency, record_session_start_latency, record_spawn_latency,
};
use super::super::runtime_state::{
    clear_runtime, remember_runtime_caps, remember_runtime_config_options, remember_runtime_models,
    remember_runtime_modes, LearnedCaps,
};
use super::super::transport::{run_io, AcpConnection};
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

/// Everything a cold start launches with. Owned, because concurrent callers share one start
/// as a `'static` future.
pub(crate) struct ColdStartSpec {
    pub(crate) runtime_key: &'static str,
    pub(crate) role_name: String,
    pub(crate) app_session_id: String,
    pub(crate) binary: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) cwd: String,
    pub(crate) auto_approve: bool,
    pub(crate) mcp_servers: Vec<acp::McpServer>,
    pub(crate) resume_session_id: Option<String>,
    /// The role's instructions; sent only when the agent advertises support.
    pub(crate) system_prompt: Option<String>,
}

/// The session request's `systemPrompt`: the role's instructions in append mode, only when
/// the agent advertised `sessionCapabilities.systemPrompt`. An agent must reject a session
/// request carrying a system prompt it cannot apply.
pub(super) fn session_system_prompt(
    advertised: bool,
    text: Option<&str>,
) -> Option<acp::SystemPrompt> {
    let text = text
        .map(str::trim)
        .filter(|t| advertised && !t.is_empty())?;
    Some(acp::SystemPrompt::new(vec![acp::ContentBlock::from(
        text.to_string(),
    )]))
}

pub(crate) async fn cold_start(spec: ColdStartSpec) -> Result<LiveConnection, String> {
    let ColdStartSpec {
        runtime_key,
        role_name,
        app_session_id,
        binary,
        args,
        env: env_pairs,
        cwd: abs_cwd,
        auto_approve,
        mcp_servers,
        resume_session_id,
        system_prompt: requested_prompt,
    } = spec;
    let (role_name, app_session_id, binary, abs_cwd) = (
        role_name.as_str(),
        app_session_id.as_str(),
        binary.as_str(),
        abs_cwd.as_str(),
    );
    clear_runtime(runtime_key);
    let delta_slot: DeltaSlot = Arc::new(Mutex::new(None));
    acp_log("spawn.start", json!({ "binary": binary, "cwd": abs_cwd }));

    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(&args)
        .current_dir(abs_cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    for (k, v) in &env_pairs {
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
    let subagents: Rc<RefCell<super::super::subagents::Subagents>> = Default::default();
    let tool_activity: Rc<RefCell<super::super::activity::ToolActivity>> = Default::default();
    let client = Rc::new(JockeyUiClient {
        delta_slot: delta_slot.clone(),
        auto_approve,
        runtime_key: runtime_key.to_string(),
        role_name: role_name.to_string(),
        app_session_id: app_session_id.to_string(),
        expected_session_id: expected_session_id.clone(),
        mode_state: mode_state.clone(),
        config_state: config_state.clone(),
        terminals,
        subagents: subagents.clone(),
        tool_activity: tool_activity.clone(),
    });
    let conn = Rc::new(AcpConnection::new(stdin));
    let io_conn = conn.clone();
    let io_client = client.clone();

    let (health_tx, mut health_rx) = tokio::sync::watch::channel(true);
    let io_handle = tokio::task::spawn_local(async move {
        if let Err(e) = run_io(stdout, io_conn, io_client).await {
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
                        .subagents(acp::SubagentCapabilities::new())
                        .plan(acp::PlanCapabilities::new())
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
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(AcpLayerError::process_crashed(
                            &format!("initialize ({status})"),
                            &stderr_tail(&stderr_buf),
                        )
                        .into_message());
                    }
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

    let providers_rev = crate::llm_providers::revision(runtime_key);
    super::providers::apply_saved_providers(&conn, runtime_key, &init_resp).await;

    let t = Instant::now();
    let supports_load = init_resp.agent_capabilities.load_session;
    // Not part of stable ACP: claude-agent-acp and codex-acp agree on a `_session/steering`
    // extension and advertise it under the initialize response's `_meta`.
    let steering = init_resp
        .meta
        .as_ref()
        .and_then(|m| m.get("steering"))
        .and_then(|s| s.get("supported"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let supports_fork = init_resp
        .agent_capabilities
        .session_capabilities
        .fork
        .is_some();
    if remember_runtime_caps(
        runtime_key,
        LearnedCaps {
            steering,
            fork: supports_fork,
        },
    ) {
        crate::assistant::invalidate_assistant_catalog();
    }
    // `session/resume` reattaches without replaying history. Agents that keep sessions but
    // cannot replay them (DeepSeek Harness) advertise only this.
    let supports_resume = init_resp
        .agent_capabilities
        .session_capabilities
        .resume
        .is_some();

    let system_prompt = session_system_prompt(
        init_resp
            .agent_capabilities
            .session_capabilities
            .system_prompt
            .is_some(),
        requested_prompt.as_deref(),
    );

    struct SessionStartResult {
        session_id: acp::SessionId,
        config_options: Option<Vec<acp::SessionConfigOption>>,
        modes: Option<acp::SessionModeState>,
        resumed: bool,
    }

    async fn do_new_session(
        conn: &AcpConnection,
        abs_cwd: &str,
        mcp_servers: Vec<acp::McpServer>,
        system_prompt: Option<acp::SystemPrompt>,
        stderr_buf: Arc<Mutex<String>>,
    ) -> Result<SessionStartResult, String> {
        let mut req = acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd))
            .system_prompt(system_prompt);
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

    if resume_session_id.is_some() && !supports_load && !supports_resume {
        return Err("This ACP adapter cannot load or resume the existing session. The transcript and handle were preserved; use an adapter with session/load or session/resume support or explicitly reset context.".into());
    }
    let start_result: SessionStartResult = if let Some(sid) = resume_session_id
        .clone()
        .filter(|_| !supports_load && supports_resume)
    {
        let mut resume_req = acp::ResumeSessionRequest::new(
            acp::SessionId::from(sid.clone()),
            std::path::PathBuf::from(abs_cwd),
        )
        .system_prompt(system_prompt.clone());
        if !mcp_servers.is_empty() {
            resume_req = resume_req.mcp_servers(mcp_servers.clone());
        }
        let resumed: Result<acp::ResumeSessionResponse, acp::Error> =
            tokio::time::timeout(SESSION_TIMEOUT, conn.request("session/resume", resume_req))
                .await
                .map_err(|_| {
                    acp::Error::new(
                        acp::ErrorCode::InternalError.into(),
                        format!(
                            "timeout: resume_session exceeded {}s",
                            SESSION_TIMEOUT.as_secs()
                        ),
                    )
                })
                .and_then(|r| r);
        match resumed {
            Ok(resp) => {
                acp_log(
                    "session.resume.ok",
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
                return Err(format!("Could not resume the existing ACP session. Its transcript and handle were preserved; retry or explicitly reset the agent context. Details: {}", AcpLayerError::from(e).into_message()));
            }
        }
    } else if supports_load {
        if let Some(sid) = resume_session_id {
            let mut load_req = acp::LoadSessionRequest::new(
                acp::SessionId::from(sid.clone()),
                std::path::PathBuf::from(abs_cwd),
            )
            .system_prompt(system_prompt.clone());
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
                    return Err(format!("Could not resume the existing ACP session. Its transcript and handle were preserved; retry or explicitly reset the agent context. Details: {}", AcpLayerError::from(e).into_message()));
                }
            }
        } else {
            do_new_session(
                &conn,
                abs_cwd,
                mcp_servers,
                system_prompt.clone(),
                stderr_buf.clone(),
            )
            .await?
        }
    } else {
        do_new_session(
            &conn,
            abs_cwd,
            mcp_servers,
            system_prompt.clone(),
            stderr_buf.clone(),
        )
        .await?
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
                "ids": serialized.iter().filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect::<Vec<_>>(),
                "options": serialized.iter().map(|v| {
                    json!({
                        "id": v.get("id"),
                        "name": v.get("name"),
                        "category": v.get("category"),
                        "currentValue": v.get("currentValue"),
                        "optionCount": v.get("options").and_then(|opts| opts.as_array()).map(|opts| opts.len()).unwrap_or(0)
                    })
                }).collect::<Vec<_>>()
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
    remember_runtime_modes(runtime_key, available_modes_ids.clone());
    acp_log(
        "modes.discovered",
        json!({
            "runtime": runtime_key,
            "count": available_modes_ids.len(),
            "ids": available_modes_ids,
            "current": start_result.modes.as_ref().map(|m| m.current_mode_id.to_string())
        }),
    );

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
        conn,
        session_id,
        cwd: abs_cwd.to_string(),
        env: env_pairs,
        delta_slot,
        mode_state,
        config_state,
        child_pid: child.id(),
        steering,
        providers_rev,
        system_prompt: requested_prompt,
        subagents,
        tool_activity,
        last_active: Instant::now(),
        _child: child,
        _io_task: io_handle,
        health_rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_sent_only_when_advertised_and_non_empty() {
        assert!(session_system_prompt(false, Some("Be terse.")).is_none());
        assert!(session_system_prompt(true, None).is_none());
        assert!(session_system_prompt(true, Some("  \n")).is_none());
        let built = session_system_prompt(true, Some("  Be terse.\n")).expect("advertised");
        let json = serde_json::to_value(&built).unwrap();
        assert_eq!(json.to_string().matches("Be terse.").count(), 1);
        assert!(!json.to_string().contains("\\n"));
    }
}
