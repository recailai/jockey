//! ACP connection abstraction.
//!
//! Consumers of the worker connection pool depend on [`AgentConnection`] and
//! [`AgentRpc`] rather than the concrete [`super::worker::LiveConnection`]
//! type. `LiveConnection` is the sole production implementation today; test
//! stubs and alternate transports can slot in behind these traits without
//! touching the worker dispatch loop.
//!
//! Split into two traits deliberately:
//!
//! - [`AgentConnection`] exposes **state**: ids, cells, cwd, health rx, etc.
//!   These are accessors over fields that live behind a `RefCell` borrow on
//!   the thread-local `CONN_MAP`. They are cheap (mostly clones of `Rc`s).
//! - [`AgentRpc`] exposes **operations**: `prompt`, `cancel`, `set_session_mode`,
//!   `set_session_config_option`. Returned as an `Rc<dyn AgentRpc>` handle from
//!   [`AgentConnection::rpc_handle`] so the caller can release its `CONN_MAP`
//!   borrow *before* awaiting an RPC future.

use agent_client_protocol::{self as acp};
use async_trait::async_trait;
use std::rc::Rc;
use std::time::Instant;
use tokio::sync::watch;

use super::worker::{ConfigStateCell, DeltaSlot, ModeStateCell};

#[async_trait(?Send)]
pub(crate) trait AgentRpc {
    async fn prompt(&self, req: acp::PromptRequest) -> Result<acp::PromptResponse, acp::Error>;

    async fn cancel(&self, note: acp::CancelNotification);

    async fn set_session_mode(
        &self,
        req: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error>;

    async fn set_session_config_option(
        &self,
        req: acp::SetSessionConfigOptionRequest,
    ) -> Result<acp::SetSessionConfigOptionResponse, acp::Error>;
}

#[async_trait(?Send)]
impl AgentRpc for acp::ClientSideConnection {
    async fn prompt(&self, req: acp::PromptRequest) -> Result<acp::PromptResponse, acp::Error> {
        <acp::ClientSideConnection as acp::Agent>::prompt(self, req).await
    }

    async fn cancel(&self, note: acp::CancelNotification) {
        let _ = <acp::ClientSideConnection as acp::Agent>::cancel(self, note).await;
    }

    async fn set_session_mode(
        &self,
        req: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        <acp::ClientSideConnection as acp::Agent>::set_session_mode(self, req).await
    }

    async fn set_session_config_option(
        &self,
        req: acp::SetSessionConfigOptionRequest,
    ) -> Result<acp::SetSessionConfigOptionResponse, acp::Error> {
        <acp::ClientSideConnection as acp::Agent>::set_session_config_option(self, req).await
    }
}

#[allow(dead_code)]
pub(crate) trait AgentConnection {
    fn instance_id(&self) -> u64;
    fn session_id(&self) -> acp::SessionId;
    fn cwd(&self) -> &str;
    fn child_pid(&self) -> Option<u32>;
    fn delta_slot(&self) -> DeltaSlot;
    fn mode_state(&self) -> ModeStateCell;
    fn config_state(&self) -> ConfigStateCell;
    fn health_rx(&self) -> watch::Receiver<bool>;
    fn last_active(&self) -> Instant;
    fn touch_last_active(&mut self);

    /// A cloneable RPC handle that outlives any `CONN_MAP` borrow.
    /// Callers grab it, drop the borrow, then `.await` on it.
    fn rpc_handle(&self) -> Rc<dyn AgentRpc>;
}
