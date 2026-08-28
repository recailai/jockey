pub(crate) mod adapter_runtime;
mod cold_start;
mod execute;
mod headless;
mod mcp;
mod native;
pub(crate) mod perm_bridge;
mod prewarm;
mod session_cmds;

// cold_start is called by worker/handlers.rs via super::super::session::cold_start
pub(crate) use cold_start::cold_start;

pub use execute::execute_runtime;

/// True when the runtime supports provider session administration
/// (list/fork/rewind). Verified per-protocol; native Codex only.
pub(crate) fn supports_provider_sessions(runtime_kind: &str) -> bool {
    use adapter_runtime::AnyRuntimeAdapter;
    matches!(
        AnyRuntimeAdapter::resolve(runtime_kind),
        Some(AnyRuntimeAdapter::Native {
            protocol: super::adapter::NativeProtocol::CodexAppServer,
        })
    )
}

/// True when the runtime supports importing a persisted provider session
/// handle. Codex resumes via `thread/resume`; Pi resumes via `--session <id>`.
pub(crate) fn supports_session_import(runtime_kind: &str) -> bool {
    use adapter_runtime::AnyRuntimeAdapter;
    matches!(
        AnyRuntimeAdapter::resolve(runtime_kind),
        Some(AnyRuntimeAdapter::Native { .. })
    )
}
#[allow(unused_imports)]
pub(crate) use headless::{reclaim_idle_headless_sessions, shutdown_headless_sessions};
pub(crate) use native::codex_admin::{
    fork_codex_thread, list_codex_threads, ProviderThreadSummary,
};
#[allow(unused_imports)]
pub(crate) use native::{
    reclaim_idle_native_sessions, rollback_live_codex_thread, shutdown_native_sessions,
};
pub use prewarm::{prewarm_role, prewarm_role_with_session_id, refresh_role_config_defs};
pub use session_cmds::{
    cancel_session, reconnect_session, reset_session, set_config_option, set_mode, sync_role_mode,
};
