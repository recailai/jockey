use agent_client_protocol::{self as acp};
use serde_json::{json, Value};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

use super::super::adapter::{acp_log, resolve_cwd};
use super::super::error::{AcpErrorCode, AcpLayerError};
use super::super::metrics::{record_error, record_idle_reclaim, record_prompt_latency};
use super::super::runtime_state::{
    list_discovered_config_options, list_discovered_modes, remember_runtime_available_commands,
    remember_runtime_modes,
};
use super::super::session::cold_start;
use super::notify::{notify_connection_death, notify_prewarm};
use super::pool::{
    child_pids, pool_key, CANCEL_HANDLES, CONN_MAP, DELTA_CHANNEL_CAPACITY, PENDING_COLD_STARTS,
    PROMPT_LOCKS, PROMPT_WAITERS,
};
use futures::future::FutureExt;
use super::types::{AcpEvent, ConnectionDeathEvent, PrewarmStatus};
use super::{cancel_all_permissions, cancel_permissions_for};

const PROMPT_LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const IDLE_RECLAIM_AFTER: std::time::Duration = std::time::Duration::from_secs(300);

/// Emit a structured SessionError event on the delta stream so the UI can
/// render a recovery action before the final result_tx rejection arrives.
/// Best-effort: if the receiver is already gone the event is silently dropped.
pub(crate) fn emit_session_error(
    delta_tx: &mpsc::Sender<AcpEvent>,
    code: AcpErrorCode,
    message: impl Into<String>,
    retryable: bool,
) {
    let _ = delta_tx.try_send(AcpEvent::SessionError {
        code: code.as_str().to_string(),
        message: message.into(),
        retryable,
    });
}

/// Recover `(AcpErrorCode, retryable)` from a string that went through
/// `AcpLayerError::into_message()` (format: `"{CODE}: {msg}{ (retryable)?}"`).
/// Returns `None` if the prefix doesn't look like a known code.
fn parse_layer_error_prefix(msg: &str) -> Option<(AcpErrorCode, bool)> {
    let code_str = msg.split(':').next()?;
    let code = match code_str {
        "AUTH_REQUIRED" => AcpErrorCode::AuthRequired,
        "CONNECTION_FAILED" => AcpErrorCode::ConnectionFailed,
        "PROCESS_CRASHED" => AcpErrorCode::ProcessCrashed,
        "PROMPT_TIMEOUT" => AcpErrorCode::PromptTimeout,
        "ACP_REQ_CANCELLED" => AcpErrorCode::RequestCancelled,
        "INVALID_ACP_REQUEST" => AcpErrorCode::InvalidRequest,
        "ACP_INVALID_PARAMS" => AcpErrorCode::InvalidParams,
        "ACP_METHOD_NOT_FOUND" => AcpErrorCode::MethodNotFound,
        "AGENT_SESSION_NOT_FOUND" => AcpErrorCode::ResourceNotFound,
        "INTERNAL_ERROR" => AcpErrorCode::InternalError,
        "AGENT_ERROR" => AcpErrorCode::AgentError,
        _ => return None,
    };
    let retryable = msg.contains("(retryable)");
    Some((code, retryable))
}

// ── Connection lifecycle ──────────────────────────────────────────────────────

pub(crate) async fn shutdown_worker_state() {
    // Cancel all in-progress prompts
    let cancel_items = CANCEL_HANDLES.with(|m| {
        let mut map = m.borrow_mut();
        let items = map
            .iter()
            .map(|(k, h)| (k.clone(), h.conn.clone(), h.session_id.clone()))
            .collect::<Vec<_>>();
        map.clear();
        items
    });
    for (_key, conn, session_id) in cancel_items {
        conn.cancel(acp::CancelNotification::new(session_id)).await;
    }

    // Drop all live connections
    CONN_MAP.with(|m| {
        m.borrow_mut().clear();
    });
    PROMPT_LOCKS.with(|m| {
        m.borrow_mut().clear();
    });
    PROMPT_WAITERS.with(|m| {
        m.borrow_mut().clear();
    });
    PENDING_COLD_STARTS.with(|m| {
        m.borrow_mut().clear();
    });

    use super::super::runtime_state::clear_all as clear_runtime_state;
    clear_runtime_state();

    cancel_all_permissions();

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
        json!({ "forceKilled": remaining_pids.len() }),
    );
}

pub(crate) fn reclaim_idle_connections() {
    let now = std::time::Instant::now();
    let active_prompt_keys = CANCEL_HANDLES.with(|m| {
        m.borrow()
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
    });
    let stale_keys = CONN_MAP.with(|m| {
        m.borrow()
            .iter()
            .filter_map(|(key, conn)| {
                (!active_prompt_keys.contains(key)
                    && now.duration_since(conn.last_active) >= IDLE_RECLAIM_AFTER)
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>()
    });
    for key in stale_keys {
        acp_log("pool.idle_reclaim", json!({ "key": key }));
        if let Some(runtime_key) = key.split(':').nth(1) {
            record_idle_reclaim(runtime_key);
        }
        CONN_MAP.with(|m| m.borrow_mut().remove(&key));
    }
}

pub(crate) async fn reset_worker_session(
    runtime_key: &'static str,
    role_name: &str,
    app_session_id: &str,
) -> Result<(), String> {
    let key = pool_key(app_session_id, runtime_key, role_name);

    let cancel = CANCEL_HANDLES.with(|m| m.borrow_mut().remove(&key));
    if let Some(h) = cancel {
        h.conn
            .cancel(acp::CancelNotification::new(h.session_id))
            .await;
    }

    CONN_MAP.with(|m| m.borrow_mut().remove(&key));
    cancel_permissions_for(runtime_key, role_name, app_session_id);
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

    let cancel = CANCEL_HANDLES.with(|m| m.borrow_mut().remove(&key));
    if let Some(h) = cancel {
        h.conn
            .cancel(acp::CancelNotification::new(h.session_id))
            .await;
    }

    CONN_MAP.with(|m| m.borrow_mut().remove(&key));
    cancel_permissions_for(runtime_key, role_name, app_session_id);
    use super::super::runtime_state::clear_session as clear_session_runtime_state;
    clear_session_runtime_state(app_session_id, runtime_key, role_name);
    acp_log(
        "reconnect.ok",
        json!({ "runtime": runtime_key, "role": role_name, "appSession": app_session_id }),
    );
    Ok(())
}

// ── Connection helpers ────────────────────────────────────────────────────────

/// Evict the connection if the working directory has changed.
pub(crate) fn evict_if_cwd_changed(key: &str, resolved: &str, runtime_key: &str, role_name: &str) {
    let should_evict = CONN_MAP.with(|m| {
        m.borrow()
            .get(key)
            .map(|c| c.cwd != resolved)
            .unwrap_or(false)
    });
    if should_evict {
        acp_log(
            "pool.evict",
            json!({ "runtime": runtime_key, "role": role_name, "reason": "cwd_change" }),
        );
        CONN_MAP.with(|m| m.borrow_mut().remove(key));
    }
}

/// Ensure a live connection exists for the given key; cold-start if missing.
/// Returns `true` if a cold start was performed (or shared with a concurrent
/// cold start). Multiple concurrent callers for the same key share a single
/// `cold_start` task via `PENDING_COLD_STARTS`.
pub(crate) async fn ensure_connection(
    key: &str,
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
    let already_live = CONN_MAP.with(|m| m.borrow().contains_key(key));
    if already_live {
        CONN_MAP.with(|m| {
            if let Some(conn) = m.borrow_mut().get_mut(key) {
                conn.last_active = Instant::now();
            }
        });
        acp_log(
            "pool.reuse",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
        return Ok(false);
    }

    // If another task is already cold-starting this key, await its future.
    let pending = PENDING_COLD_STARTS.with(|m| m.borrow().get(key).cloned());
    if let Some(shared) = pending {
        acp_log(
            "pool.cold_start.shared",
            json!({ "runtime": runtime_key, "role": role_name }),
        );
        shared.await?;
        return Ok(true);
    }

    // We are the first; build the cold_start future, insert it as Shared, then
    // await it. On completion, insert the resulting LiveConnection into
    // CONN_MAP and remove the pending entry.
    let key_owned = key.to_string();
    let role_owned = role_name.to_string();
    let app_session_owned = app_session_id.to_string();
    let binary_owned = binary.to_string();
    let args_owned = args.to_vec();
    let env_owned = env.to_vec();
    let resolved_owned = resolved.to_string();
    let pending_key_for_task = key_owned.clone();
    let task = async move {
        let result = cold_start(
            runtime_key,
            &role_owned,
            &app_session_owned,
            &binary_owned,
            &args_owned,
            &env_owned,
            &resolved_owned,
            auto_approve,
            mcp_servers,
            resume_session_id,
        )
        .await;
        // Always clear the pending entry, regardless of success.
        PENDING_COLD_STARTS.with(|m| {
            m.borrow_mut().remove(&pending_key_for_task);
        });
        match result {
            Ok(conn) => {
                let instance_id = conn.instance_id;
                acp_log(
                    "pool.cold_start",
                    json!({
                        "runtime": runtime_key,
                        "role": role_owned,
                        "sessionId": conn.session_id.to_string()
                    }),
                );
                CONN_MAP.with(|m| m.borrow_mut().insert(pending_key_for_task.clone(), conn));
                Ok(instance_id)
            }
            Err(e) => Err(e),
        }
    }
    .boxed_local()
    .shared();

    PENDING_COLD_STARTS.with(|m| {
        m.borrow_mut().insert(key_owned, task.clone());
    });

    task.await?;
    Ok(true)
}

pub(crate) async fn apply_cold_start_config(
    key: &str,
    session_id: &acp::SessionId,
    delta_tx: &mpsc::Sender<AcpEvent>,
    role_mode: &Option<String>,
    role_config_options: &[(String, String)],
    runtime_key: &str,
) {
    let mode_snapshot: Option<acp::SessionModeState> = CONN_MAP.with(|m| {
        m.borrow()
            .get(key)
            .and_then(|c| c.mode_state.borrow().clone())
    });

    if let Some(state) = &mode_snapshot {
        let modes_json: Vec<Value> = state
            .available_modes
            .iter()
            .filter_map(|mode| serde_json::to_value(mode).ok())
            .collect();
        if !modes_json.is_empty() {
            let _ = delta_tx.try_send(AcpEvent::AvailableModes {
                modes: modes_json,
                current: Some(state.current_mode_id.to_string()),
            });
        }
    }

    let conn_rc = CONN_MAP.with(|m| m.borrow().get(key).map(|c| crate::acp::AgentConnection::rpc_handle(c)));
    let Some(conn_rc) = conn_rc else { return };

    if let Some(mode) = role_mode {
        // Pre-validate: if the saved mode isn't in the agent's advertised list,
        // skip the RPC and surface an UNSUPPORTED_CONFIG warning so the user
        // knows why their saved configuration didn't take effect.
        let is_supported = mode_snapshot
            .as_ref()
            .map(|state| {
                state
                    .available_modes
                    .iter()
                    .any(|m| m.id.to_string() == *mode)
            })
            .unwrap_or(true); // no snapshot yet — let the agent decide
        if !is_supported {
            let msg = format!(
                "saved role mode '{mode}' is not advertised by runtime '{runtime_key}'"
            );
            acp_log(
                "config.unsupported",
                json!({ "mode": mode, "runtime": runtime_key, "reason": "not in available_modes" }),
            );
            emit_session_error(delta_tx, AcpErrorCode::InvalidParams, msg, false);
        } else if let Err(e) = conn_rc
            .set_session_mode(acp::SetSessionModeRequest::new(
                session_id.clone(),
                acp::SessionModeId::from(mode.clone()),
            ))
            .await
        {
            let layer = AcpLayerError::from(e);
            acp_log(
                "config.set_mode.error",
                json!({ "mode": mode, "error": layer.message }),
            );
            emit_session_error(
                delta_tx,
                layer.code,
                format!("failed to set mode '{mode}'"),
                layer.retryable,
            );
        }
    }

    let discovered = list_discovered_config_options(runtime_key);
    let supported_keys: std::collections::HashSet<String> = discovered
        .iter()
        .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .collect();

    for (k, value) in role_config_options {
        if !supported_keys.is_empty() && !supported_keys.contains(k.as_str()) {
            let msg = format!(
                "saved role config '{k}' is not advertised by runtime '{runtime_key}'"
            );
            acp_log(
                "config.unsupported",
                json!({ "key": k, "runtime": runtime_key, "reason": "not in discovered options" }),
            );
            emit_session_error(delta_tx, AcpErrorCode::InvalidParams, msg, false);
            continue;
        }
        if let Err(e) = conn_rc
            .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                session_id.clone(),
                acp::SessionConfigId::from(k.clone()),
                acp::SessionConfigValueId::from(value.clone()),
            ))
            .await
        {
            let layer = AcpLayerError::from(e);
            acp_log(
                "config.set_option.error",
                json!({ "key": k, "error": layer.message }),
            );
            emit_session_error(
                delta_tx,
                layer.code,
                format!("failed to apply config '{k}'"),
                layer.retryable,
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

/// Spawn a task that watches the health channel and evicts the connection when
/// the agent process dies.
pub(crate) fn spawn_connection_health_watch(
    key: String,
    instance_id: u64,
    mut death: ConnectionDeathEvent,
    mut hrx: tokio::sync::watch::Receiver<bool>,
) {
    tokio::task::spawn_local(async move {
        let reason = loop {
            if hrx.changed().await.is_err() {
                break "connection health watcher closed".to_string();
            }
            if !*hrx.borrow() {
                break "agent io stopped".to_string();
            }
        };
        acp_log("health.process_died", json!({ "key": &key }));

        // Only evict if the connection in the map is still the same instance.
        let is_same = CONN_MAP.with(|m| {
            m.borrow()
                .get(&key)
                .map(|c| c.instance_id == instance_id)
                .unwrap_or(false)
        });

        if is_same {
            CONN_MAP.with(|m| m.borrow_mut().remove(&key));
            CANCEL_HANDLES.with(|m| m.borrow_mut().remove(&key));
            death.reason = Some(reason);
            cancel_permissions_for(&death.runtime_key, &death.role_name, &death.app_session_id);
            notify_connection_death(death);
        }
    });
}

// ── handle_execute ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_execute(
    key: String,
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
    let resolved = resolve_cwd(&cwd);

    evict_if_cwd_changed(&key, &resolved, runtime_key, &role_name);

    let conn_exists = CONN_MAP.with(|m| m.borrow().contains_key(&key));
    if !conn_exists {
        let _ = delta_tx.try_send(AcpEvent::StatusUpdate {
            text: "Connecting to agent runtime...".to_string(),
        });
    }

    let is_cold = match ensure_connection(
        &key,
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
            record_error(runtime_key);
            // Parse the code out of the serialized AcpLayerError string (cold_start
            // returns `AcpLayerError::into_message()`). If parsing fails we fall
            // back to a generic connection failure which is the most common cold
            // start outcome.
            let (code, retryable) = parse_layer_error_prefix(&e)
                .unwrap_or((AcpErrorCode::ConnectionFailed, true));
            emit_session_error(&delta_tx, code, e.clone(), retryable);
            let _ = result_tx.send(Err(e));
            return;
        }
    };

    let live = CONN_MAP.with(|m| {
        m.borrow().get(&key).map(|c| {
            (
                c.session_id.clone(),
                c.delta_slot.clone(),
                crate::acp::AgentConnection::rpc_handle(c),
                c.health_rx.clone(),
                c.instance_id,
            )
        })
    });
    let Some((session_id, delta_slot, conn_rc, mut health_rx, instance_id)) = live else {
        let _ = result_tx.send(Err("connection disappeared after cold start".to_string()));
        return;
    };

    if let Ok(mut slot_guard) = delta_slot.lock() {
        *slot_guard = Some(delta_tx.clone());
    }

    if is_cold {
        let health_rx_clone = health_rx.clone();
        apply_cold_start_config(
            &key,
            &session_id,
            &delta_tx,
            &role_mode,
            &role_config_options,
            runtime_key,
        )
        .await;

        let death = ConnectionDeathEvent {
            runtime_key: runtime_key.to_string(),
            role_name: role_name.clone(),
            app_session_id: app_session_id.clone(),
            reason: None,
        };
        spawn_connection_health_watch(key.clone(), instance_id, death, health_rx_clone);
    } else {
        let mode_snapshot: Option<acp::SessionModeState> = CONN_MAP.with(|m| {
            m.borrow()
                .get(&key)
                .and_then(|c| c.mode_state.borrow().clone())
        });
        if let Some(state) = mode_snapshot {
            let modes_json: Vec<Value> = state
                .available_modes
                .iter()
                .filter_map(|mode| serde_json::to_value(mode).ok())
                .collect();
            if !modes_json.is_empty() {
                let _ = delta_tx.try_send(AcpEvent::AvailableModes {
                    modes: modes_json,
                    current: Some(state.current_mode_id.to_string()),
                });
            }
        }
    }

    let blocks = build_prompt_blocks(prompt, &context);

    // Install cancel handle so the Cancel message handler can send ACP cancel
    // without touching CONN_MAP.
    CANCEL_HANDLES.with(|m| {
        m.borrow_mut().insert(
            key.clone(),
            super::pool::CancelHandle {
                conn: conn_rc.clone(),
                session_id: session_id.clone(),
            },
        );
    });

    let prompt_lock = PROMPT_LOCKS.with(|m| {
        let mut map = m.borrow_mut();
        map.entry(key.clone())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    });
    let queue_position = PROMPT_WAITERS.with(|m| {
        let mut map = m.borrow_mut();
        let entry = map.entry(key.clone()).or_insert(0);
        let current = *entry;
        *entry += 1;
        current
    });
    let prompt_guard = match prompt_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let _ = delta_tx.try_send(AcpEvent::StatusUpdate {
                text: format!(
                    "Waiting for previous turn... queue position {}",
                    queue_position + 1
                ),
            });
            prompt_lock.lock().await
        }
    };
    PROMPT_WAITERS.with(|m| {
        let mut map = m.borrow_mut();
        if let Some(count) = map.get_mut(&key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&key);
            }
        }
    });

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
        m.borrow_mut().remove(&key);
    });
    drop(prompt_guard);

    if let Ok(mut slot_guard) = delta_slot.lock() {
        *slot_guard = None;
    }

    match prompt_result {
        Ok(Ok(resp)) => {
            record_prompt_latency(runtime_key, prompt_started.elapsed().as_millis());
            CONN_MAP.with(|m| {
                if let Some(conn) = m.borrow_mut().get_mut(&key) {
                    conn.last_active = Instant::now();
                }
            });
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
            record_error(runtime_key);
            acp_log(
                "pool.invalidate",
                json!({ "runtime": runtime_key, "error": e.to_string() }),
            );
            let layer = AcpLayerError::from(e.clone());
            emit_session_error(&delta_tx, layer.code, layer.message.clone(), layer.retryable);
            // Invalidate connection only if still the same instance.
            let is_same = CONN_MAP.with(|m| {
                m.borrow()
                    .get(&key)
                    .map(|c| c.instance_id == instance_id)
                    .unwrap_or(false)
            });
            if is_same {
                CONN_MAP.with(|m| m.borrow_mut().remove(&key));
                cancel_permissions_for(runtime_key, &role_name, &app_session_id);
                notify_connection_death(ConnectionDeathEvent {
                    runtime_key: runtime_key.to_string(),
                    role_name: role_name.clone(),
                    app_session_id: app_session_id.clone(),
                    reason: Some(e.to_string()),
                });
            }
            let _ = result_tx.send(Err(e.to_string()));
        }
        Err(reason) => {
            record_error(runtime_key);
            acp_log(
                "pool.prompt.process_died",
                json!({
                    "runtime": runtime_key,
                    "role": role_name,
                    "elapsedSec": prompt_started.elapsed().as_secs(),
                    "reason": reason
                }),
            );
            emit_session_error(
                &delta_tx,
                AcpErrorCode::ProcessCrashed,
                reason.to_string(),
                true,
            );
            conn_rc
                .cancel(acp::CancelNotification::new(session_id.clone()))
                .await;
            let is_same = CONN_MAP.with(|m| {
                m.borrow()
                    .get(&key)
                    .map(|c| c.instance_id == instance_id)
                    .unwrap_or(false)
            });
            if is_same {
                CONN_MAP.with(|m| m.borrow_mut().remove(&key));
                cancel_permissions_for(runtime_key, &role_name, &app_session_id);
                notify_connection_death(ConnectionDeathEvent {
                    runtime_key: runtime_key.to_string(),
                    role_name: role_name.clone(),
                    app_session_id: app_session_id.clone(),
                    reason: Some(reason.to_string()),
                });
            }
            let _ = result_tx.send(Err(reason.to_string()));
        }
    }
}

// ── handle_prewarm ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_prewarm(
    key: String,
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
) {
    let already_live = CONN_MAP.with(|m| m.borrow().contains_key(&key));
    if already_live {
        CONN_MAP.with(|m| {
            if let Some(conn) = m.borrow_mut().get_mut(&key) {
                conn.last_active = Instant::now();
            }
        });
        if force_refresh {
            acp_log(
                "prewarm.force_refresh",
                json!({ "runtime": runtime_key, "role": role_name }),
            );
            CONN_MAP.with(|m| m.borrow_mut().remove(&key));
        } else {
            if let Some(tx) = result_tx {
                let session_id = CONN_MAP.with(|m| {
                    m.borrow()
                        .get(&key)
                        .map(|c| c.session_id.to_string())
                        .unwrap_or_default()
                });
                let _ = tx.send((
                    list_discovered_config_options(runtime_key),
                    list_discovered_modes(runtime_key),
                    session_id,
                ));
            }
            return;
        }
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
            let instance_id = conn.instance_id;
            let health_rx_clone = conn.health_rx.clone();

            let dummy_tx = {
                let (tx, _rx) = mpsc::channel::<AcpEvent>(DELTA_CHANNEL_CAPACITY);
                tx
            };

            CONN_MAP.with(|m| m.borrow_mut().insert(key.clone(), conn));

            apply_cold_start_config(
                &key,
                &session_id,
                &dummy_tx,
                &role_mode,
                &role_config_options,
                runtime_key,
            )
            .await;

            // Install a temporary drain channel so notifications sent after session/new
            // (e.g. AvailableCommandsUpdate via setTimeout) are captured rather than dropped.
            let (drain_tx, mut drain_rx) = mpsc::channel::<AcpEvent>(DELTA_CHANNEL_CAPACITY);
            let delta_slot = CONN_MAP.with(|m| m.borrow().get(&key).map(|c| c.delta_slot.clone()));
            if let Some(ds) = delta_slot {
                if let Ok(mut sg) = ds.lock() {
                    *sg = Some(drain_tx);
                }
            }

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
            drop(drain_rx);

            // Clear the drain channel from the delta_slot
            let delta_slot = CONN_MAP.with(|m| {
                m.borrow()
                    .get(&key)
                    .filter(|c| c.instance_id == instance_id)
                    .map(|c| c.delta_slot.clone())
            });
            if let Some(ds) = delta_slot {
                if let Ok(mut sg) = ds.lock() {
                    *sg = None;
                }
            }

            let death = ConnectionDeathEvent {
                runtime_key: runtime_key.to_string(),
                role_name: role_name.clone(),
                app_session_id: app_session_id.clone(),
                reason: None,
            };
            spawn_connection_health_watch(key, instance_id, death, health_rx_clone);

            if let Some(tx) = result_tx {
                let _ = tx.send((
                    list_discovered_config_options(runtime_key),
                    list_discovered_modes(runtime_key),
                    session_id_str,
                ));
            }
        }
        Err(e) => {
            record_error(runtime_key);
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
