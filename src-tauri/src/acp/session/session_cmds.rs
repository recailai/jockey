use tokio::sync::oneshot;

use super::super::worker::{worker_tx, WorkerMsg};
use super::adapter_runtime::{AnyRuntimeAdapter, RuntimeAdapter, SessionKey};

fn normalize_runtime_key(runtime_kind: &str) -> Option<&'static str> {
    crate::runtime_profile::runtime_key_static(runtime_kind)
}

fn resolve_session_id(app_session_id: Option<&str>) -> Option<String> {
    app_session_id
        .filter(|id| !id.trim().is_empty())
        .map(|id| id.to_string())
}

fn session_key(runtime_key: &'static str, role_name: &str, app_session_id: &str) -> SessionKey {
    SessionKey::new(runtime_key, role_name, app_session_id)
}

pub async fn cancel_session(runtime_kind: &str, role_name: &str, app_session_id: Option<&str>) {
    let Some(runtime_key) = normalize_runtime_key(runtime_kind) else {
        return;
    };
    let Some(resolved_session_id) = resolve_session_id(app_session_id) else {
        return;
    };
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    match AnyRuntimeAdapter::resolve(runtime_key) {
        // Slot-based runtimes cancel in place; no drain handshake exists.
        Some(adapter) if !matches!(adapter, AnyRuntimeAdapter::AcpWorker) => {
            RuntimeAdapter::cancel(&adapter, &key);
        }
        // ACP (and unknown runtimes): fire the cancel through the worker and
        // wait for the in-flight prompt to drain (bounded inside the worker).
        // Frontend awaits this so it knows the old turn is fully done before
        // sending a queued message.
        _ => {
            let (tx, rx) = oneshot::channel();
            if worker_tx()
                .send(WorkerMsg::Cancel {
                    runtime_key,
                    role_name: role_name.to_string(),
                    app_session_id: resolved_session_id,
                    result_tx: Some(tx),
                })
                .is_err()
            {
                return;
            }
            let _ = rx.await;
        }
    }
}

pub async fn reset_session(
    runtime_kind: &str,
    role_name: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let resolved_session_id =
        resolve_session_id(app_session_id).ok_or_else(|| "app session id required".to_string())?;
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    let adapter =
        AnyRuntimeAdapter::resolve(runtime_key).ok_or_else(|| "adapter unavailable".to_string())?;
    RuntimeAdapter::discard_slot(&adapter, &key).await
}

pub async fn reconnect_session(
    runtime_kind: &str,
    role_name: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let resolved_session_id =
        resolve_session_id(app_session_id).ok_or_else(|| "app session id required".to_string())?;
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    let adapter =
        AnyRuntimeAdapter::resolve(runtime_key).ok_or_else(|| "adapter unavailable".to_string())?;
    RuntimeAdapter::reconnect_slot(&adapter, &key).await
}

pub async fn set_mode(
    runtime_kind: &str,
    role_name: &str,
    mode_id: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let resolved_session_id =
        resolve_session_id(app_session_id).ok_or_else(|| "app session id required".to_string())?;
    if !matches!(
        AnyRuntimeAdapter::resolve(runtime_key),
        Some(AnyRuntimeAdapter::AcpWorker)
    ) {
        // Native/headless runtimes have no live mode switching.
        return Ok(());
    }
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetMode {
        runtime_key,
        role_name: role_name.to_string(),
        app_session_id: resolved_session_id,
        mode_id: mode_id.to_string(),
        result_tx: tx,
    });
    rx.await.map_err(|_| "worker disconnected".to_string())?
}

pub async fn sync_role_mode(
    role_name: &str,
    mode_id: &str,
    eligible_session_ids: Vec<String>,
) -> Vec<String> {
    if eligible_session_ids.is_empty() {
        return vec![];
    }
    let (tx, rx) = oneshot::channel();
    if worker_tx()
        .send(WorkerMsg::SyncRoleMode {
            role_name: role_name.to_string(),
            mode_id: mode_id.to_string(),
            eligible_session_ids,
            result_tx: tx,
        })
        .is_err()
    {
        return vec![];
    }
    rx.await.unwrap_or_default()
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
    let resolved_session_id =
        resolve_session_id(app_session_id).ok_or_else(|| "app session id required".to_string())?;
    if !matches!(
        AnyRuntimeAdapter::resolve(runtime_key),
        Some(AnyRuntimeAdapter::AcpWorker)
    ) {
        // Native/headless runtimes have no live config switching.
        return Ok(());
    }
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::SetConfigOption {
        runtime_key,
        role_name: role_name.to_string(),
        app_session_id: resolved_session_id,
        config_id: key.to_string(),
        value: value.to_string(),
        result_tx: tx,
    });
    rx.await.map_err(|_| "worker disconnected".to_string())?
}
