use tokio::sync::oneshot;

use super::super::worker::{worker_tx, WorkerMsg};
use super::adapter_runtime::{AdapterError, AnyRuntimeAdapter, RuntimeAdapter, SessionKey};

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

pub async fn cancel_session(
    runtime_kind: &str,
    role_name: &str,
    app_session_id: Option<&str>,
) -> Result<(), String> {
    let Some(runtime_key) = normalize_runtime_key(runtime_kind) else {
        return Ok(());
    };
    let Some(resolved_session_id) = resolve_session_id(app_session_id) else {
        return Ok(());
    };
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    match AnyRuntimeAdapter::resolve(runtime_key) {
        Some(AnyRuntimeAdapter::Headless { .. }) => {
            super::headless::cancel_headless_and_wait(runtime_key, role_name, &resolved_session_id)
                .await
        }
        Some(AnyRuntimeAdapter::Native { .. }) => {
            super::native::cancel_native_and_wait(runtime_key, role_name, &resolved_session_id)
                .await
        }
        Some(adapter) if !matches!(adapter, AnyRuntimeAdapter::AcpWorker) => {
            RuntimeAdapter::cancel(&adapter, &key).map_err(AdapterError::into)
        }
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
                return Err("ACP worker is unavailable".to_string());
            }
            rx.await
                .map_err(|_| "ACP cancel acknowledgement was dropped".to_string())
        }
    }
}

pub async fn steer_session(
    runtime_kind: &str,
    role_name: &str,
    app_session_id: Option<&str>,
    prompt: &str,
    attachments: &[crate::types::ImageAttachment],
) -> Result<(), String> {
    let runtime_key =
        normalize_runtime_key(runtime_kind).ok_or_else(|| "unsupported runtime".to_string())?;
    let resolved_session_id =
        resolve_session_id(app_session_id).ok_or_else(|| "app session id required".to_string())?;
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    let adapter =
        AnyRuntimeAdapter::resolve(runtime_key).ok_or_else(|| "adapter unavailable".to_string())?;
    RuntimeAdapter::steer(&adapter, &key, prompt, attachments)
        .await
        .map_err(AdapterError::into)
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
    RuntimeAdapter::discard_slot(&adapter, &key)
        .await
        .map_err(AdapterError::into)
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
    RuntimeAdapter::reconnect_slot(&adapter, &key)
        .await
        .map_err(AdapterError::into)
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
    let key = session_key(runtime_key, role_name, &resolved_session_id);
    let adapter =
        AnyRuntimeAdapter::resolve(runtime_key).ok_or_else(|| "adapter unavailable".to_string())?;
    RuntimeAdapter::set_mode(&adapter, &key, mode_id)
        .await
        .map_err(AdapterError::into)
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
    let session_k = session_key(runtime_key, role_name, &resolved_session_id);
    let adapter =
        AnyRuntimeAdapter::resolve(runtime_key).ok_or_else(|| "adapter unavailable".to_string())?;
    RuntimeAdapter::set_config_option(&adapter, &session_k, key, value)
        .await
        .map_err(AdapterError::into)
}
