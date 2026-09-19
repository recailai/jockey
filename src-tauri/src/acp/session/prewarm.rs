use serde_json::Value;
use tokio::sync::oneshot;

use super::super::adapter::{build_stdio_adapter, AdapterTransport};
use super::super::worker::{worker_tx, WorkerMsg};
use super::headless::refresh_headless_catalog;
use super::mcp::load_role_mcp_servers;
use super::native::refresh_native_catalog;
use crate::acp::protocol as acp;
use crate::db::app_session_role::{load_app_session_role_cli_id, save_app_session_role_cli_id};
use crate::db::role::update_role_config_option_defs_if_changed;
use crate::types::AppState;

fn normalize_runtime_key(runtime_kind: &str) -> Option<&'static str> {
    crate::runtime_profile::runtime_key_static(runtime_kind)
}

// ── Internal shared implementation ───────────────────────────────────────────

struct PrewarmOpts<'a> {
    runtime_kind: &'a str,
    role_name: &'a str,
    cwd: &'a str,
    resume_session_id: Option<String>,
    app_session_id: Option<&'a str>,
    mcp_servers: Vec<acp::McpServer>,
    role_mode: Option<String>,
    role_config_options: Vec<(String, String)>,
    force_refresh: bool,
    /// The caller will bind the returned provider session id, so the native path has to spawn
    /// even when its catalog is already known.
    want_session_id: bool,
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
    if let AdapterTransport::HeadlessJson { protocol, .. } = adapter.transport {
        // Headless CLIs have no control plane to hold open, so the catalog is the whole
        // prewarm: there is no provider session id to bind.
        let catalog = refresh_headless_catalog(
            protocol,
            adapter.runtime_key,
            &adapter.binary,
            &adapter.env,
            opts.cwd,
            opts.force_refresh,
        )
        .await;
        let _ = tx.send((catalog.options, catalog.modes, String::new()));
        return Some(rx);
    }
    if let AdapterTransport::Native(protocol) = adapter.transport {
        let catalog = refresh_native_catalog(
            protocol,
            adapter.runtime_key,
            &adapter.binary,
            &adapter.args,
            &adapter.env,
            opts.cwd,
            &opts.role_config_options,
            opts.resume_session_id.clone(),
            true,
            // Callers that bind a provider session id (Pi resume) must actually spawn, so they
            // set `want_session_id`; catalog-only callers may reuse what was already read.
            opts.force_refresh || opts.want_session_id,
        )
        .await;
        let _ = tx.send((catalog.options, catalog.modes, catalog.session_id));
        return Some(rx);
    }
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

fn persist_config_option_defs(
    state: &AppState,
    role_name: &str,
    runtime_kind: &str,
    opts: &[Value],
) {
    if opts.is_empty() {
        return;
    }
    match serde_json::to_string(opts) {
        Ok(serialized) => {
            if let Err(e) = update_role_config_option_defs_if_changed(
                state,
                role_name,
                Some(runtime_kind),
                &serialized,
            ) {
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
    persist_config_option_defs(state, role_name, runtime_key, opts);
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
    project_id: Option<&str>,
) -> (Option<String>, Vec<(String, String)>) {
    let Ok(Some(role)) = crate::db::role::load_role_scoped(state, role_name, project_id) else {
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
    project_id: Option<&'a str>,
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
        .map(|s| {
            let project_id = req.app_session_id.and_then(|app_session_id| {
                crate::db::app_session::get_app_session_project_id(s, app_session_id)
            });
            load_role_mcp_servers(
                s,
                req.role_name,
                project_id.as_deref().or(req.project_id),
                req.runtime_kind,
            )
        })
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
        want_session_id: req.persist_cli_id,
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
            persist_config_option_defs(s, req.role_name, req.runtime_kind, &opts);
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
        .map(|(s, app_session_id)| {
            let project_id = crate::db::app_session::get_app_session_project_id(s, app_session_id);
            load_role_mcp_servers(s, role_name, project_id.as_deref(), runtime_kind)
        })
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
        want_session_id: true,
    })
    .await
    else {
        return;
    };

    if let (Some((s, app_sid)), Ok((opts, _modes, sid))) = (state, rx.await) {
        persist_prewarm_result(s, app_sid, runtime_key, role_name, &opts, &sid);
    }
}

/// Config option definitions only (no session ID involved).
///
/// `force` re-reads the catalog from the binary; otherwise the process-lifetime catalog is
/// reused. This distinction is the whole point of the short-circuit: persona and agent
/// switches call this constantly, and forcing there spawned a `codex app-server` /
/// `pi --mode rpc` (or a `claude`/`agy` probe) every single time. Only an explicit "refresh
/// models" action should pay that cost.
pub async fn refresh_role_config_defs_with(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    state: &AppState,
    project_id: Option<&str>,
    force: bool,
) -> (Vec<Value>, Vec<String>) {
    let (role_mode, role_config_options) = load_role_default_config(state, role_name, project_id);
    prewarm_config_impl(ConfigPrewarmRequest {
        runtime_kind,
        role_name,
        cwd,
        state: Some(state),
        app_session_id: None,
        project_id,
        resume_session_id: None,
        role_mode,
        role_config_options,
        force_refresh: force,
        persist_cli_id: false,
    })
    .await
}

/// Reuses the discovered catalog when there is one. This is the hot path (every persona and
/// agent switch); use `refresh_role_config_defs_with(.., true)` for an explicit refresh.
pub async fn refresh_role_config_defs(
    runtime_kind: &str,
    role_name: &str,
    cwd: &str,
    state: &AppState,
) -> (Vec<Value>, Vec<String>) {
    refresh_role_config_defs_with(runtime_kind, role_name, cwd, state, None, false).await
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
    let project_id = crate::db::app_session::get_app_session_project_id(state, app_session_id);
    let mcp_servers = load_role_mcp_servers(state, role_name, project_id.as_deref(), runtime_kind);
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
        want_session_id: true,
    })
    .await
    else {
        return;
    };
    if let Ok((opts, _modes, sid)) = rx.await {
        persist_prewarm_result(state, app_session_id, runtime_key, role_name, &opts, &sid);
    }
}
