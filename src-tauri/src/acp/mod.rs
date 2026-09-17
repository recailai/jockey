mod adapter;
mod client;
mod connection;
mod error;
mod metrics;
pub(crate) mod process;
pub(crate) mod protocol;
mod runtime_state;
pub(crate) mod session;
mod transport;
mod worker;

pub(crate) use connection::{AgentConnection, AgentRpc};

pub use adapter::{
    acp_log_snapshot, adapter_launch_method, adapter_transport, clear_adapter_cache, probe_runtime,
    resolve_adapter_launch, set_app_data_dir, AcpLogEntry,
};
pub use metrics::{snapshot as metrics_snapshot, AcpRuntimeMetrics};
pub use runtime_state::{
    clear_discovered_catalogs, list_available_commands, list_discovered_models,
};
pub use session::perm_bridge::bridge_subcommand_main as permission_bridge_main;
pub(crate) use session::perm_bridge::start_permission_bridge;
pub use session::{
    cancel_session, execute_runtime, prewarm_role, prewarm_role_with_session_id, reconnect_session,
    refresh_role_config_defs, refresh_role_config_defs_with, reset_session, set_config_option,
    set_mode, sync_role_mode,
};
pub use worker::{
    active_connections_snapshot, respond_to_permission, respond_to_user_input,
    set_death_event_sender, set_prewarm_event_sender, shutdown, ActiveConnectionInfo,
    ConnectionDeathEvent, PrewarmEvent,
};
