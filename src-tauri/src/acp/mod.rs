mod adapter;
mod client;
mod connection;
mod error;
mod metrics;
mod runtime_state;
mod session;
mod worker;

pub(crate) use connection::{AgentConnection, AgentRpc};

pub use adapter::{
    acp_log_snapshot, clear_adapter_cache, probe_runtime, set_app_data_dir, AcpLogEntry,
};
pub use metrics::{snapshot as metrics_snapshot, AcpRuntimeMetrics};
pub use runtime_state::{
    list_available_commands, list_discovered_config_options, list_discovered_models,
    list_discovered_modes,
};
pub use session::{
    cancel_session, execute_runtime, prewarm_role, prewarm_role_for_config,
    prewarm_role_with_session_id, reconnect_session, refresh_role_config_defs, reset_session,
    set_config_option, set_mode,
};
pub use worker::{
    active_connections_snapshot, respond_to_permission, set_death_event_sender,
    set_prewarm_event_sender, shutdown, ActiveConnectionInfo, ConnectionDeathEvent, PrewarmEvent,
};
