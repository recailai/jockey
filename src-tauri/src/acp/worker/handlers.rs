use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::super::adapter::{acp_log, resolve_cwd};
use super::super::runtime_state::{
    list_discovered_config_options, list_discovered_modes, remember_runtime_available_commands,
    remember_runtime_modes,
};
use super::super::session::cold_start;
use super::notify::{notify_connection_death, notify_prewarm};
use super::pool::{
    pool_key, slot_map, child_pids, LiveConnection, SlotHandle, CANCEL_HANDLES,
    DELTA_CHANNEL_CAPACITY,
};
use super::types::{AcpEvent, ConnectionDeathEvent, PrewarmStatus};

const PROMPT_LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

// ── Connection lifecycle ──────────────────────────────────────────────────────

pub(crate) async fn shutdown_worker_state() {
    let cancel_handles = CANCEL_HANDLES.with(|m| {
        let mut map = m.borrow_mut();
        let items = map
            .iter()
            .map(|(k, h)| (k.clone(), h.conn.clone(), h.session_id.clone()))
            .collect::<Vec<_>>();
        map.clear();
        items
    });
    for (_key, conn, session_id) in cancel_handles {
        let _ = conn.cancel(acp::CancelNotification::new(session_id)).await;
    }

    let slots: Vec<(String, Arc<SlotHandle>)> = slot_map()
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect();
    let mut dropped = 0usize;
    let mut timed_out = 0usize;
    for (_key, slot) in &slots {
        match tokio::time::timeout(std::time::Duration::from_millis(1200), slot.conn.lock()).await {
            Ok(mut guard) => {
                *guard = None;
                dropped += 1;
            }
            Err(_) => {
                timed_out += 1;
            }
        }
    }
    slot_map().clear();

    use super::super::runtime_state::clear_all as clear_runtime_state;
    clear_runtime_state();

    use super::permission::permission_requests;
    let request_ids: Vec<String> = permission_requests()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    for request_id in request_ids {
        if let Some((_, tx)) = permission_requests().remove(&request_id) {
            let _ = tx.send(acp::RequestPermissionOutcome::Cancelled);
        }
    }

    use super::super::client::shutdown_terminals;
    shutdown_terminals().await;

    let remaining_pids: Vec<u32> = child_pids().iter().map(|r| *r).collect();
    for pid in &remaining_pids {
        unsafe {
            libc::kill(-(*pid as i32), libc::SIGKILL);
        }
        child_pids().remove(pid);
    }

    acp_log(
        "shutdown.complete",
        json!({ "slots": slots.len(), "dropped": dropped, "timedOut": timed_out, "forceKilled": remaining_pids.len() }),
    );
}

pub(crate) async fn reset_worker_session(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    let key = pool_key(app_session_id, runtime_key, role_name);
    let handle = CANCEL_HANDLES.with(|m| m.borrow_mut().remove(&key));
    if let Some(h) = handle {
        let _ = h
            .conn
            .cancel(acp::CancelNotification::new(h.session_id))
            .await;
    }

    if let Some((_, slot)) = slot_map().remove(&key) {
        let mut guard =
            tokio::time::timeout(std::time::Duration::from_millis(1200), slot.conn.lock())
                .await
                .map_err(|_| "timeout resetting active ACP session".to_string())?;
        *guard = None;
    }

    use super::super::runtime_state::clear_session as clear_session_runtime_state;
    clear_session_runtime_state(app_session_id, runtime_key, role_name);
    Ok(())
}

pub(crate) async fn reconnect_worker_session(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    let key = pool_key(app_session_id, runtime_key, role_name);
    let handle = CANCEL_HANDLES.with(|m| m.borrow_mut().remove(&key));
    if let Some(h) = handle {
        let _ = h
            .conn
            .cancel(acp::CancelNotification::new(h.session_id))
            .await;
    }

    if let Some((_, slot)) = slot_map().remove(&key) {
        let mut guard =
            tokio::time::timeout(std::time::Duration::from_millis(1200), slot.conn.lock())
                .await
                .map_err(|_| "timeout reconnecting ACP session".to_string())?;
        *guard = None;
    }

    use super::super::runtime_state::clear_session as clear_session_runtime_state;
    clear_session_runtime_state(app_session_id, runtime_key, role_name);
    acp_log(
        "reconnect.ok",
        json!({ "runtime": runtime_key, "role": role_name, "appSession": app_session_id }),
    );
    Ok(())
}

// ── Connection helpers ────────────────────────────────────────────────────────

pub(crate) async fn evict_if_cwd_changed(
    guard: &mut Option<LiveConnection>,
    resolved: &str,
    runtime_key: &str,
    role_name: &str,
) {
    if guard.as_ref().map(|c| c.cwd != resolved).unwrap_or(false) {
        acp_log(
            "pool.evict",
            json!({ "runtime": runtime_key, "role": role_name, "reason": "cwd_change" }),
        );
        *guard = None;
    }
}

pub(crate) async fn ensure_connection(
    guard: &mut Option<LiveConnection>,
    runtime_key: &'static str,
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    resolved: &str,
    role_name: &str,
    app_session_id: &str,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    resume_session_id: Option<String>,
) -> Result<bool, String> {
    if guard.is_some() {
        acp_log(
            "pool.reuse",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
        return Ok(false);
    }
    let conn = cold_start(
        runtime_key,
        role_name,
        app_session_id,
        binary,
        args,
        env,
        resolved,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await?;
    acp_log(
        "pool.cold_start",
        json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
    );
    *guard = Some(conn);
    Ok(true)
}

pub(crate) async fn apply_cold_start_config(
    conn: &LiveConnection,
    session_id: &acp::SessionId,
    delta_tx: &mpsc::Sender<AcpEvent>,
    role_mode: &Option<String>,
    role_config_options: &[(String, String)],
    _app_session_id: &str,
    runtime_key: &str,
    _role_name: &str,
) {
    if !conn.available_modes.is_empty() {
        let _ = delta_tx.try_send(AcpEvent::AvailableModes {
            modes: conn.available_modes.clone(),
            current: conn.current_mode.clone(),
        });
    }

    if let Some(mode) = role_mode {
        if let Err(e) = conn
            .conn
            .set_session_mode(acp::SetSessionModeRequest::new(
                session_id.clone(),
                acp::SessionModeId::from(mode.clone()),
            ))
            .await
        {
            acp_log(
                "config.set_mode.error",
                json!({ "mode": mode, "error": e.to_string() }),
            );
        }
    }

    let discovered = list_discovered_config_options(runtime_key);
    let supported_keys: std::collections::HashSet<String> = discovered
        .iter()
        .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .collect();

    for (key, value) in role_config_options {
        if !supported_keys.is_empty() && !supported_keys.contains(key.as_str()) {
            acp_log(
                "config.set_option.skipped",
                json!({ "key": key, "runtime": runtime_key, "reason": "not supported by runtime" }),
            );
            continue;
        }
        if let Err(e) = conn
            .conn
            .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                session_id.clone(),
                acp::SessionConfigId::from(key.clone()),
                acp::SessionConfigValueId::from(value.clone()),
            ))
            .await
        {
            acp_log(
                "config.set_option.error",
                json!({ "key": key, "error": e.to_string() }),
            );
        }
    }
}

pub(crate) fn build_prompt_blocks(
    prompt: String,
    context: &[(String, String)],
) -> Vec<acp::ContentBlock> {
    let mut blocks: Vec<acp::ContentBlock> =
        vec![acp::ContentBlock::Text(acp::TextContent::new(prompt))];
    if !context.is_empty() {
        let ctx = context
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(format!(
            "[context]\n{ctx}"
        ))));
    }
    blocks
}

pub(crate) fn spawn_connection_health_watch(
    slot: Arc<SlotHandle>,
    wk: String,
    death: ConnectionDeathEvent,
    instance_id: u64,
    mut hrx: tokio::sync::watch::Receiver<bool>,
) {
    tokio::task::spawn_local(async move {
        loop {
            if hrx.changed().await.is_err() {
                break;
            }
            if !*hrx.borrow() {
                break;
            }
        }
        acp_log("health.process_died", json!({ "key": &wk }));

        let mut should_notify = false;
        {
            let mut g = slot.conn.lock().await;
            let is_same_connection = g
                .as_ref()
                .map(|live| live.instance_id == instance_id)
                .unwrap_or(false);
            if is_same_connection {
                *g = None;
                should_notify = true;
            }
        }

        if should_notify {
            CANCEL_HANDLES.with(|m| {
                m.borrow_mut().remove(&wk);
            });
            notify_connection_death(death);
        }
    });
}

// ── handle_execute ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_execute(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    app_session_id: String,
    prompt: String,
    context: Vec<(String, String)>,
    cwd: String,
    delta_tx: mpsc::Sender<AcpEvent>,
    result_tx: oneshot::Sender<Result<(String, String), String>>,
    auto_approve: bool,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    resume_session_id: Option<String>,
) {
    let mut guard = slot.conn.lock().await;
    let resolved = resolve_cwd(&cwd);

    evict_if_cwd_changed(&mut guard, &resolved, runtime_key, &role_name).await;

    if guard.is_none() {
        let _ = delta_tx.try_send(AcpEvent::StatusUpdate {
            text: "Connecting to agent runtime...".to_string(),
        });
    }

    let is_cold = match ensure_connection(
        &mut guard,
        runtime_key,
        &binary,
        &args,
        &env,
        &resolved,
        &role_name,
        &app_session_id,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await
    {
        Ok(cold) => cold,
        Err(e) => {
            let _ = result_tx.send(Err(e));
            return;
        }
    };

    let session_id = guard
        .as_ref()
        .map(|c| c.session_id.clone())
        .unwrap_or_else(|| acp::SessionId::from(String::new()));
    let delta_slot = guard
        .as_ref()
        .map(|c| c.delta_slot.clone())
        .unwrap_or_else(|| Arc::new(Mutex::new(None)));

    if let Ok(mut slot_guard) = delta_slot.lock() {
        *slot_guard = Some(delta_tx.clone());
    }

    // Extract everything we need from the guard before releasing the mutex.
    // Holding `slot.conn` locked across the entire prompt() call prevents any
    // concurrent operation on the same slot (SetMode, health eviction, a second
    // message arriving) from making progress — which is the root cause of the
    // "stuck message" and "same-role deadlock" bugs.
    let conn_rc = guard.as_ref().unwrap().conn.clone();
    let mut health_rx = guard.as_ref().unwrap().health_rx.clone();
    let instance_id = guard.as_ref().unwrap().instance_id;

    if is_cold {
        let conn = guard.as_ref().unwrap();
        apply_cold_start_config(
            conn,
            &session_id,
            &delta_tx,
            &role_mode,
            &role_config_options,
            &app_session_id,
            runtime_key,
            &role_name,
        )
        .await;

        let wk = pool_key(&app_session_id, runtime_key, &role_name);
        let death = ConnectionDeathEvent {
            runtime_key: runtime_key.to_string(),
            role_name: role_name.clone(),
            app_session_id: app_session_id.clone(),
        };
        spawn_connection_health_watch(slot.clone(), wk, death, instance_id, conn.health_rx.clone());
    } else if let Some(conn) = guard.as_ref() {
        if !conn.available_modes.is_empty() {
            let _ = delta_tx.try_send(AcpEvent::AvailableModes {
                modes: conn.available_modes.clone(),
                current: conn.current_mode.clone(),
            });
        }
    }

    let blocks = build_prompt_blocks(prompt, &context);

    // Install cancel handle so the Cancel message handler can send ACP cancel
    // without waiting for the conn mutex.
    let cancel_key = pool_key(&app_session_id, runtime_key, &role_name);
    CANCEL_HANDLES.with(|m| {
        m.borrow_mut().insert(
            cancel_key.clone(),
            super::pool::CancelHandle {
                conn: conn_rc.clone(),
                session_id: session_id.clone(),
            },
        );
    });

    // Release the conn mutex before awaiting prompt() so that other tasks
    // (SetMode, health watch eviction, a queued second message) can acquire
    // it while the AI is thinking.
    drop(guard);

    // Serialize prompt() calls on this slot: ACP sessions are single-request
    // streams; a second prompt must wait until the first completes.
    let _prompt_guard = slot.prompt_lock.lock().await;

    let prompt_started = Instant::now();
    let prompt_fut = conn_rc.prompt(acp::PromptRequest::new(session_id.clone(), blocks));
    tokio::pin!(prompt_fut);

    let prompt_result: Result<Result<acp::PromptResponse, acp::Error>, &str> = loop {
        tokio::select! {
            res = &mut prompt_fut => {
                break Ok(res);
            }
            changed = health_rx.changed() => {
                if changed.is_err() || !*health_rx.borrow() {
                    break Err("agent process exited while prompt was in progress");
                }
            }
            _ = tokio::time::sleep(PROMPT_LIVENESS_INTERVAL) => {
                if !*health_rx.borrow() {
                    break Err("agent process is no longer alive");
                }
            }
        }
    };

    CANCEL_HANDLES.with(|m| {
        m.borrow_mut().remove(&cancel_key);
    });

    if let Ok(mut slot_guard) = delta_slot.lock() {
        *slot_guard = None;
    }

    match prompt_result {
        Ok(Ok(resp)) => {
            acp_log(
                "stage.ok",
                json!({
                    "runtime": runtime_key,
                    "stage": "session/prompt",
                    "latencyMs": prompt_started.elapsed().as_millis(),
                    "stopReason": format!("{:?}", resp.stop_reason)
                }),
            );
            let _ = result_tx.send(Ok((String::new(), session_id.to_string())));
        }
        Ok(Err(e)) => {
            acp_log(
                "pool.invalidate",
                json!({ "runtime": runtime_key, "error": e.to_string() }),
            );
            // Re-acquire the slot mutex only to invalidate the connection.
            let mut guard = slot.conn.lock().await;
            // Only clear if this is still the same connection instance we used.
            if guard.as_ref().map(|c| c.instance_id == instance_id).unwrap_or(false) {
                *guard = None;
            }
            let _ = result_tx.send(Err(e.to_string()));
        }
        Err(reason) => {
            acp_log(
                "pool.prompt.process_died",
                json!({
                    "runtime": runtime_key,
                    "role": role_name,
                    "elapsedSec": prompt_started.elapsed().as_secs(),
                    "reason": reason
                }),
            );
            let _ = conn_rc
                .cancel(acp::CancelNotification::new(session_id.clone()))
                .await;
            let mut guard = slot.conn.lock().await;
            if guard.as_ref().map(|c| c.instance_id == instance_id).unwrap_or(false) {
                *guard = None;
            }
            let _ = result_tx.send(Err(reason.to_string()));
        }
    }
}

// ── handle_prewarm ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_prewarm(
    slot: Arc<SlotHandle>,
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
) {
    let mut guard = slot.conn.lock().await;
    if guard.is_some() {
        if let Some(tx) = result_tx {
            let session_id = guard
                .as_ref()
                .map(|c| c.session_id.to_string())
                .unwrap_or_default();
            let _ = tx.send((
                list_discovered_config_options(runtime_key),
                list_discovered_modes(runtime_key),
                session_id,
            ));
        }
        return;
    }
    let resolved = resolve_cwd(&cwd);
    acp_log(
        "prewarm.start",
        json!({ "runtime": runtime_key, "role": role_name }),
    );
    notify_prewarm(
        runtime_key,
        &role_name,
        &app_session_id,
        PrewarmStatus::Started,
    );
    match cold_start(
        runtime_key,
        &role_name,
        &app_session_id,
        &binary,
        &args,
        &env,
        &resolved,
        auto_approve,
        mcp_servers,
        resume_session_id,
    )
    .await
    {
        Ok(conn) => {
            acp_log(
                "prewarm.ok",
                json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }),
            );
            notify_prewarm(
                runtime_key,
                &role_name,
                &app_session_id,
                PrewarmStatus::Ready,
            );
            let session_id = conn.session_id.clone();
            let session_id_str = session_id.to_string();
            let dummy_tx = {
                let (tx, _rx) = mpsc::channel::<AcpEvent>(DELTA_CHANNEL_CAPACITY);
                tx
            };
            apply_cold_start_config(
                &conn,
                &session_id,
                &dummy_tx,
                &role_mode,
                &role_config_options,
                &app_session_id,
                runtime_key,
                &role_name,
            )
            .await;
            // Install a temporary drain channel so notifications sent after session/new
            // (e.g. AvailableCommandsUpdate via setTimeout) are captured rather than dropped.
            let (drain_tx, mut drain_rx) = mpsc::channel::<AcpEvent>(DELTA_CHANNEL_CAPACITY);
            {
                if let Ok(mut slot_guard) = conn.delta_slot.lock() {
                    *slot_guard = Some(drain_tx);
                }
            }
            let instance_id = conn.instance_id;
            let health_rx_clone = conn.health_rx.clone();
            *guard = Some(conn);
            // Release the conn mutex before the 300 ms drain so that an Execute
            // message arriving during prewarm can acquire the slot immediately
            // instead of blocking until the drain deadline.
            drop(guard);

            // Drain for up to 300 ms to collect any immediate notifications.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(300);
            loop {
                match tokio::time::timeout_at(deadline, drain_rx.recv()).await {
                    Ok(Some(AcpEvent::AvailableCommands { commands })) => {
                        acp_log(
                            "commands.discovered",
                            json!({
                                "runtime": runtime_key,
                                "role": role_name,
                                "count": commands.len(),
                                "names": commands.iter().filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()
                            }),
                        );
                        remember_runtime_available_commands(
                            &app_session_id,
                            runtime_key,
                            &role_name,
                            commands,
                        );
                    }
                    Ok(Some(AcpEvent::AvailableModes { modes, .. })) => {
                        remember_runtime_modes(
                            runtime_key,
                            modes
                                .iter()
                                .filter_map(|m| {
                                    m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
                                })
                                .collect(),
                        );
                    }
                    Ok(Some(_)) => {}
                    _ => break,
                }
            }
            // Drop drain channel then clear the delta_slot via the live connection.
            drop(drain_rx);
            {
                let g = slot.conn.lock().await;
                if let Some(live) = g.as_ref() {
                    if live.instance_id == instance_id {
                        if let Ok(mut slot_guard) = live.delta_slot.lock() {
                            *slot_guard = None;
                        }
                    }
                }
            }
            let wk = pool_key(&app_session_id, runtime_key, &role_name);
            let death = ConnectionDeathEvent {
                runtime_key: runtime_key.to_string(),
                role_name: role_name.clone(),
                app_session_id: app_session_id.clone(),
            };
            spawn_connection_health_watch(slot.clone(), wk, death, instance_id, health_rx_clone);
            if let Some(tx) = result_tx {
                let _ = tx.send((
                    list_discovered_config_options(runtime_key),
                    list_discovered_modes(runtime_key),
                    session_id_str,
                ));
            }
        }
        Err(e) => {
            acp_log(
                "prewarm.error",
                json!({ "runtime": runtime_key, "role": role_name, "error": e }),
            );
            notify_prewarm(
                runtime_key,
                &role_name,
                &app_session_id,
                PrewarmStatus::Failed { error: e.clone() },
            );
            if let Some(tx) = result_tx {
                let _ = tx.send((vec![], vec![], String::new()));
            }
        }
    }
}
