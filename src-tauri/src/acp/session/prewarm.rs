use serde_json::Value;
use tokio::sync::oneshot;

use super::super::adapter::build_stdio_adapter;
use super::super::worker::RuntimeKind;
use super::super::worker::{worker_tx, WorkerMsg};
use super::mcp::load_role_mcp_servers;
use crate::db::app_session_role::{load_app_session_role_cli_id, save_app_session_role_cli_id};
use crate::db::role::update_role_config_option_defs_if_changed;
use crate::types::AppState;

fn normalize_runtime_key(runtime_kind: &str) -> Option<&'static str> {
    RuntimeKind::from_str(runtime_kind).map(|k| k.runtime_key())
}

// ── Internal shared implementation ───────────────────────────────────────────

struct PrewarmOpts<'a> {
    runtime_kind: &'a str,
    role_name: &'a str,
    cwd: &'a str,
    resume_session_id: Option<String>,
    app_session_id: Option<&'a str>,
    mcp_servers: Vec<agent_client_protocol::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    force_refresh: bool,
}

async fn send_prewarm(
    opts: PrewarmOpts<'_>,
) -> Option<oneshot::Receiver<(Vec<Value>, Vec<String>, String)>> {
    let adapter = match build_stdio_adapter(opts.runtime_kind) {
        Ok(Some(a)) => a,
        _ => return None,
    };
    let resolved_session_id = opts
        .app_session_id
        .filter(|id| !id.trim().is_empty())
        .map(|id| id.to_string())
        .unwrap_or_else(|| format!("role-refresh:{}:{}", adapter.runtime_key, opts.role_name));
    let (tx, rx) = oneshot::channel();
    let _ = worker_tx().send(WorkerMsg::Prewarm {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary,
        args: adapter.args,
        env: adapter.env,
        role_name: opts.role_name.to_string(),
        app_session_id: resolved_session_id,
        cwd: opts.cwd.to_string(),
        auto_approve: true,
        mcp_servers: opts.mcp_servers,
        role_mode: opts.role_mode,
        role_config_options: opts.role_config_options,
        result_tx: Some(tx),
        resume_session_id: opts.resume_session_id,
        force_refresh: opts.force_refresh,
    });
    Some(rx)
}

fn persist_config_option_defs(state: &AppState, role_name: &str, opts: &[Value]) {
    if opts.is_empty() {
        return;
    }
    match serde_json::to_string(opts) {
        Ok(serialized) => {
            if let Err(e) = update_role_config_option_defs_if_changed(state, role_name, &serialized)
            {
                eprintln!("[prewarm] failed to persist config option defs for {role_name}: {e}");
            }
        }
        Err(e) => {
            eprintln!("[prewarm] failed to serialize config option defs for {role_name}: {e}");
        }
    }
}

fn persist_prewarm_result(
    state: &AppState,
    app_sid: &str,
    runtime_key: &str,
    role_name: &str,
    opts: &[Value],
    sid: &str,
) {
    persist_config_option_defs(state, role_name, opts);
    if !sid.is_empty() {
        let _ = save_app_session_role_cli_id(state, app_sid, runtime_key, role_name, sid);
    }
}

fn parse_config_map(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.as_object().map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .filter(|(_, v)| !v.trim().is_empty())
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn load_role_default_config(
    state: &AppState,
    role_name: &str,
) -> (Option<String>, Vec<(String, String)>) {
    let Ok(Some(role)) = crate::db::role::load_role(state, role_name) else {
        return (None, Vec::new());
    };
    let mut config = parse_config_map(&role.config_options_json);
    if let Some(model) = role.model.filter(|m| !m.trim().is_empty()) {
        config.retain(|(k, _)| k != "model");
        config.push(("model".to_string(), model));
    }
    (role.mode, config)
}

struct ConfigPrewarmRequest<'a> {
    runtime_kind: &'a str,
    role_name: &'a str,
    cwd: &'a str,
    state: Option<&'a AppState>,
    app_session_id: Option<&'a str>,
    resume_session_id: Option<String>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    force_refresh: bool,
    persist_cli_id: bool,
}

async fn prewarm_config_impl(req: ConfigPrewarmRequest<'_>) -> (Vec<Value>, Vec<String>) {
    let runtime_key = normalize_runtime_key(req.runtime_kind).unwrap_or(req.runtime_kind);
    let mcp_servers = req
        .state
        .map(|s| load_role_mcp_servers(s, req.role_name))
        .unwrap_or_default();
    let Some(rx) = send_prewarm(PrewarmOpts {
        runtime_kind: req.runtime_kind,
        role_name: req.role_name,
        cwd: req.cwd,
        resume_session_id: req.resume_session_id,
        app_session_id: req.app_session_id,
        mcp_servers,
        role_mode: req.role_mode,
        role_config_options: req.role_config_options,
        force_refresh: req.force_refresh,
    })
    .await
    else {
        return (vec![], vec![]);
    };

    let (opts, modes, sid) = rx.await.unwrap_or_default();
    if let Some(s) = req.state {
        if req.persist_cli_id {
            if let Some(app_sid) = req.app_session_id {
                persist_prewarm_result(s, app_sid, runtime_key, req.role_name, &opts, &sid);
            }
        } else {
            persist_config_option_defs(s, req.role_name, &opts);
        }
    }
    (opts, modes)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fire-and-forget prewarm; persists session ID and config option defs.
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
    let mcp_servers = state
        .as_ref()
        .map(|(s, _)| load_role_mcp_servers(s, role_name))
        .unwrap_or_default();

    let Some(rx) = send_prewarm(PrewarmOpts {
        runtime_kind,
        role_name,
        cwd,
        resume_session_id,
        app_session_id,
        mcp_servers,
        role_mode: None,
        role_config_options: vec![],
        force_refresh: false,
    })
    .await
    else {
        return;
    };

    if let (Some((s, app_sid)), Ok((opts, _modes, sid))) = (state, rx.await) {
        persist_prewarm_result(s, app_sid, runtime_key, role_name, &opts, &sid);
    }
}

/// Prewarm to refresh config option definitions only (no session ID involved).
pub async fn refresh_role_config_defs(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    state: &AppState,
) -> (Vec<Value>, Vec<String>) {
    let (role_mode, role_config_options) = load_role_default_config(state, role_name);
    prewarm_config_impl(ConfigPrewarmRequest {
        runtime_kind,
        role_name,
        cwd,
        state: Some(state),
        app_session_id: None,
        resume_session_id: None,
        role_mode,
        role_config_options,
        force_refresh: true,
        persist_cli_id: false,
    })
    .await
}

/// Prewarm with an explicit session ID (used when resuming a known session).
pub async fn prewarm_role_with_session_id(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    resume_session_id: Option<String>,
    state: &AppState,
    app_session_id: &str,
) {
    let runtime_key = normalize_runtime_key(runtime_kind).unwrap_or(runtime_kind);
    let mcp_servers = load_role_mcp_servers(state, role_name);
    let Some(rx) = send_prewarm(PrewarmOpts {
        runtime_kind,
        role_name,
        cwd,
        resume_session_id,
        app_session_id: Some(app_session_id),
        mcp_servers,
        role_mode: None,
        role_config_options: vec![],
        force_refresh: false,
    })
    .await
    else {
        return;
    };
    if let Ok((opts, _modes, sid)) = rx.await {
        persist_prewarm_result(state, app_session_id, runtime_key, role_name, &opts, &sid);
    }
}
