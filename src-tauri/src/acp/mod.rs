mod adapter;
pub(crate) use adapter::app_data_dir;
mod client;
mod connection;
mod error;
mod metrics;
pub(crate) mod process;
pub(crate) mod protocol;
mod runtime_state;
pub(crate) mod session;
pub(crate) mod subagents;
mod transport;
mod worker;

pub(crate) use connection::{AgentConnection, AgentRpc};

pub use adapter::{
    acp_log_snapshot, adapter_launch_method, adapter_transport, clear_adapter_cache, probe_runtime,
    set_app_data_dir, AcpLogEntry,
};
pub use metrics::{snapshot as metrics_snapshot, AcpRuntimeMetrics};
pub use runtime_state::{
    clear_discovered_catalogs, list_available_commands, list_discovered_models, runtime_caps,
    runtime_fork,
};
pub use session::{
    cancel_session, cancel_subagent, execute_runtime, fork_session, prewarm_role,
    reconnect_session, refresh_role_config_defs_with, reset_session, set_config_option, set_mode,
    steer_session, sync_role_mode,
};
pub use worker::{
    active_connections_snapshot, list_pending_permissions, list_pending_prompts,
    respond_to_permission, respond_to_user_input, set_death_event_sender, set_prewarm_event_sender,
    shutdown, ActiveConnectionInfo, ConnectionDeathEvent, PendingPrompts, PrewarmEvent,
};
