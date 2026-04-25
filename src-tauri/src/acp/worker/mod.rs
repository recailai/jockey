mod handlers;
mod notify;
mod permission;
mod pool;
mod types;

pub use crate::runtime_kind::RuntimeKind;
pub use notify::{set_death_event_sender, set_prewarm_event_sender};
pub use permission::respond_to_permission;
pub use types::{
    AcpEvent, AcpPromptResult, ActiveConnectionInfo, ConnectionDeathEvent, PrewarmEvent,
};

pub(crate) use permission::{
    cached_approval, cancel_all_permissions, cancel_permissions_for, insert_permission,
    permission_requests, PendingPermission,
};
pub(crate) use pool::{
    pool_key, register_child_pid, ConfigStateCell, DeltaSlot, LiveConnection, ModeStateCell,
    CANCEL_HANDLES, CONN_MAP, DELTA_CHANNEL_CAPACITY,
};
pub(crate) use types::WorkerMsg;

use handlers::{
    handle_execute, handle_prewarm, reclaim_idle_connections, reconnect_worker_session,
    reset_worker_session, shutdown_worker_state,
};
use serde_json::json;
use std::sync::OnceLock;
use tokio::sync::{mpsc, oneshot};

use super::adapter::acp_log;
use super::error::AcpLayerError;

static WORKER_TX: OnceLock<mpsc::UnboundedSender<WorkerMsg>> = OnceLock::new();

pub(crate) fn worker_tx() -> &'static mpsc::UnboundedSender<WorkerMsg> {
    WORKER_TX.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel::<WorkerMsg>();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("acp worker runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, run_worker(rx));
        });
        tx
    })
}

pub async fn shutdown() {
    use super::client::shutdown_terminals;
    let Some(tx) = WORKER_TX.get() else {
        shutdown_terminals().await;
        return;
    };
    let (done_tx, done_rx) = oneshot::channel();
    if tx.send(WorkerMsg::Shutdown { done_tx }).is_err() {
        shutdown_terminals().await;
        return;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), done_rx).await;
}

pub async fn active_connections_snapshot() -> Vec<ActiveConnectionInfo> {
    let (result_tx, result_rx) = oneshot::channel();
    if worker_tx()
        .send(WorkerMsg::SnapshotConnections { result_tx })
        .is_err()
    {
        return vec![];
    }
    result_rx.await.unwrap_or_default()
}

fn snapshot_connections_now() -> Vec<ActiveConnectionInfo> {
    let now = std::time::Instant::now();
    CONN_MAP.with(|m| {
        m.borrow()
            .iter()
            .map(|(key, conn)| {
                let mut parts = key.splitn(3, ':');
                let app_session_id = parts.next().unwrap_or_default().to_string();
                let runtime_key = parts.next().unwrap_or_default().to_string();
                let role_name = parts.next().unwrap_or_default().to_string();
                ActiveConnectionInfo {
                    key: key.clone(),
                    runtime_key,
                    role_name,
                    app_session_id,
                    acp_session_id: conn.session_id.to_string(),
                    cwd: conn.cwd.clone(),
                    child_pid: conn.child_pid,
                    idle_ms: now.duration_since(conn.last_active).as_millis(),
                    healthy: *conn.health_rx.borrow(),
                }
            })
            .collect()
    })
}

async fn run_worker(mut rx: mpsc::UnboundedReceiver<WorkerMsg>) {
    tokio::task::spawn_local(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            reclaim_idle_connections();
        }
    });
    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMsg::Execute {
                runtime_key,
                binary,
                args,
                env,
                role_name,
                app_session_id,
                prompt,
                context,
                attachments,
                cwd,
                delta_tx,
                result_tx,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
                resume_session_id,
            } => {
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    handle_execute(
                        key,
                        runtime_key,
                        binary,
                        args,
                        env,
                        role_name,
                        app_session_id,
                        prompt,
                        context,
                        attachments,
                        cwd,
                        delta_tx,
                        result_tx,
                        auto_approve,
                        mcp_servers,
                        role_mode,
                        role_config_options,
                        resume_session_id,
                    )
                    .await;
                });
            }
            WorkerMsg::Prewarm {
                runtime_key,
                binary,
                args,
                env,
                role_name,
                app_session_id,
                cwd,
                auto_approve,
                mcp_servers,
                role_mode,
                role_config_options,
                result_tx,
                resume_session_id,
                force_refresh,
            } => {
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    handle_prewarm(
                        key,
                        runtime_key,
                        binary,
                        args,
                        env,
                        role_name,
                        app_session_id,
                        cwd,
                        auto_approve,
                        mcp_servers,
                        role_mode,
                        role_config_options,
                        result_tx,
                        resume_session_id,
                        force_refresh,
                    )
                    .await;
                });
            }
            WorkerMsg::Cancel {
                runtime_key,
                role_name,
                app_session_id,
                result_tx,
            } => {
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                let handle = CANCEL_HANDLES.with(|m| {
                    m.borrow()
                        .get(&key)
                        .map(|h| (h.conn.clone(), h.session_id.clone()))
                });
                let prompt_lock = pool::PROMPT_LOCKS.with(|m| m.borrow().get(&key).cloned());
                cancel_permissions_for(runtime_key, &role_name, &app_session_id);

                if let Some((conn, session_id)) = handle {
                    acp_log(
                        "cancel.sending",
                        json!({ "runtime": runtime_key, "role": role_name }),
                    );
                    tokio::task::spawn_local(async move {
                        conn.cancel(agent_client_protocol::CancelNotification::new(session_id))
                            .await;

                        // Wait for the in-flight prompt to drain: acquire its
                        // per-slot lock. handle_execute holds this lock for the
                        // entire prompt future, so acquiring it here guarantees
                        // the cancelled turn has finished (agent returned
                        // StopReason::Cancelled or the process died).
                        //
                        // Bounded to 5s so a hung wrapper can't block the
                        // frontend indefinitely — frontend then proceeds with
                        // its queued prompt anyway; worst case PROMPT_LOCKS
                        // serializes them.
                        if let Some(lock) = prompt_lock {
                            let wait = async {
                                let _g = lock.lock().await;
                                // release immediately
                            };
                            let _ =
                                tokio::time::timeout(std::time::Duration::from_secs(5), wait).await;
                        }

                        if let Some(tx) = result_tx {
                            let _ = tx.send(());
                        }
                    });
                } else {
                    acp_log(
                        "cancel.no_active_session",
                        json!({ "runtime": runtime_key, "role": role_name }),
                    );
                    if let Some(tx) = result_tx {
                        let _ = tx.send(());
                    }
                }
            }
            WorkerMsg::Reset {
                runtime_key,
                role_name,
                app_session_id,
                result_tx,
            } => {
                tokio::task::spawn_local(async move {
                    let result =
                        reset_worker_session(runtime_key, &role_name, &app_session_id).await;
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::Reconnect {
                runtime_key,
                role_name,
                app_session_id,
                result_tx,
            } => {
                tokio::task::spawn_local(async move {
                    let result =
                        reconnect_worker_session(runtime_key, &role_name, &app_session_id).await;
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::SetMode {
                runtime_key,
                role_name,
                app_session_id,
                mode_id,
                result_tx,
            } => {
                use agent_client_protocol as acp;
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    let lookup = CONN_MAP.with(|m| {
                        m.borrow().get(&key).map(|live| {
                            (
                                crate::acp::AgentConnection::rpc_handle(live),
                                live.session_id.clone(),
                                live.mode_state.clone(),
                            )
                        })
                    });
                    let Some((conn, session_id, mode_state)) = lookup else {
                        let _ = result_tx.send(Err("no active session".to_string()));
                        return;
                    };
                    // Optimistic update: snapshot the old mode id, apply the new,
                    // roll back on RPC failure so UI does not keep a stale value.
                    let previous_mode_id = mode_state
                        .borrow()
                        .as_ref()
                        .map(|s| s.current_mode_id.clone());
                    if let Some(state) = mode_state.borrow_mut().as_mut() {
                        state.current_mode_id = acp::SessionModeId::from(mode_id.clone());
                    }
                    let result = conn
                        .set_session_mode(acp::SetSessionModeRequest::new(
                            session_id,
                            acp::SessionModeId::from(mode_id),
                        ))
                        .await
                        .map(|_| ())
                        .map_err(|e| AcpLayerError::from(e).into_message());
                    if result.is_err() {
                        if let Some(prev) = previous_mode_id {
                            if let Some(state) = mode_state.borrow_mut().as_mut() {
                                state.current_mode_id = prev;
                            }
                        }
                    }
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::SetConfigOption {
                runtime_key,
                role_name,
                app_session_id,
                config_id,
                value,
                result_tx,
            } => {
                use agent_client_protocol as acp;
                let key = pool_key(&app_session_id, runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    let lookup = CONN_MAP.with(|m| {
                        m.borrow().get(&key).map(|live| {
                            (
                                crate::acp::AgentConnection::rpc_handle(live),
                                live.session_id.clone(),
                                live.config_state.clone(),
                            )
                        })
                    });
                    let Some((conn, session_id, config_state)) = lookup else {
                        let _ = result_tx.send(Err("no active session".to_string()));
                        return;
                    };
                    // Optimistic update: mutate the select option's current_value,
                    // remember the old value so we can roll back on failure.
                    let target_config_id = acp::SessionConfigId::from(config_id.clone());
                    let new_value_id = acp::SessionConfigValueId::from(value.clone());
                    let previous_value = {
                        let mut cell = config_state.borrow_mut();
                        cell.iter_mut().find_map(|opt| {
                            if opt.id != target_config_id {
                                return None;
                            }
                            match &mut opt.kind {
                                acp::SessionConfigKind::Select(sel) => {
                                    let old = sel.current_value.clone();
                                    sel.current_value = new_value_id.clone();
                                    Some(old)
                                }
                                _ => None,
                            }
                        })
                    };
                    let result = conn
                        .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                            session_id,
                            target_config_id.clone(),
                            new_value_id,
                        ))
                        .await
                        .map(|_| ())
                        .map_err(|e| AcpLayerError::from(e).into_message());
                    if result.is_err() {
                        if let Some(prev) = previous_value {
                            let mut cell = config_state.borrow_mut();
                            if let Some(opt) =
                                cell.iter_mut().find(|opt| opt.id == target_config_id)
                            {
                                if let acp::SessionConfigKind::Select(sel) = &mut opt.kind {
                                    sel.current_value = prev;
                                }
                            }
                        }
                    }
                    let _ = result_tx.send(result);
                });
            }
            WorkerMsg::Shutdown { done_tx } => {
                acp_log("shutdown.start", json!({}));
                shutdown_worker_state().await;
                let _ = done_tx.send(());
                break;
            }
            WorkerMsg::SnapshotConnections { result_tx } => {
                let _ = result_tx.send(snapshot_connections_now());
            }
        }
    }
}
