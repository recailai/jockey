use agent_client_protocol as acp;
use dashmap::{DashMap, DashSet};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

use super::types::AcpEvent;

pub(crate) const DELTA_CHANNEL_CAPACITY: usize = 512;
pub(crate) type DeltaSlot = Arc<Mutex<Option<mpsc::Sender<AcpEvent>>>>;

// ── Pool key ──────────────────────────────────────────────────────────────────

pub(crate) fn pool_key(app_session_id: &str, runtime_key: &str, role_name: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

// ── SlotHandle: per-session connection slot ───────────────────────────────────

pub(crate) struct SlotHandle {
    pub(crate) conn: tokio::sync::Mutex<Option<LiveConnection>>,
    /// Serializes concurrent prompt() calls on the same slot.
    /// `conn` is released before prompt() so that SetMode / health eviction
    /// can run while the AI is thinking; this separate lock prevents two
    /// prompts from racing on the same ACP session.
    pub(crate) prompt_lock: tokio::sync::Mutex<()>,
}

/// A lightweight handle for sending ACP cancel without holding the conn mutex.
/// Stored in a thread-local map keyed by pool_key, only accessed on the worker
/// thread's LocalSet — so `Rc` is fine.
pub(crate) struct CancelHandle {
    pub(crate) conn: Rc<acp::ClientSideConnection>,
    pub(crate) session_id: acp::SessionId,
}

thread_local! {
    pub(crate) static CANCEL_HANDLES: RefCell<std::collections::HashMap<String, CancelHandle>> =
        RefCell::new(std::collections::HashMap::new());
}

// ── LiveConnection ────────────────────────────────────────────────────────────

pub(crate) struct LiveConnection {
    pub(crate) instance_id: u64,
    pub(crate) conn: Rc<acp::ClientSideConnection>,
    pub(crate) session_id: acp::SessionId,
    pub(crate) cwd: String,
    pub(crate) delta_slot: DeltaSlot,
    #[allow(dead_code)]
    pub(crate) available_modes: Vec<Value>,
    #[allow(dead_code)]
    pub(crate) current_mode: Option<String>,
    pub(crate) child_pid: Option<u32>,
    pub(crate) _child: tokio::process::Child,
    pub(crate) _io_task: tokio::task::JoinHandle<()>,
    pub(crate) health_rx: tokio::sync::watch::Receiver<bool>,
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        if let Some(pid) = self.child_pid {
            // kill_on_drop only targets the direct child process and may leave
            // grandchildren around (some ACP adapters fork a second binary).
            // We spawn each adapter in its own process group, so terminate
            // the whole group on connection drop to avoid orphan leaks.
            unsafe {
                let pgid = -(pid as i32);
                let _ = libc::kill(pgid, libc::SIGTERM);
                let _ = libc::kill(pgid, libc::SIGKILL);
                let _ = libc::kill(pid as i32, libc::SIGTERM);
                let _ = libc::kill(pid as i32, libc::SIGKILL);
            }
            unregister_child_pid(pid);
        }
    }
}

// SAFETY: LiveConnection is only ever created and accessed on the single-threaded
// worker LocalSet (see `run_worker`).  The DashMap / tokio::sync::Mutex wrappers
// require Send+Sync at the type level, but no actual cross-thread access occurs.
// The Rc<ClientSideConnection> is the only non-Send field; it is safe because:
// 1. cold_start() runs on the worker LocalSet and produces the Rc.
// 2. handle_execute/handle_prewarm run on the same LocalSet.
// 3. The DashMap is only mutated from run_worker (also on that thread).
unsafe impl Send for LiveConnection {}
unsafe impl Sync for LiveConnection {}

// ── Global state: slot map & child PIDs ──────────────────────────────────────

static SLOT_MAP: OnceLock<Arc<DashMap<String, Arc<SlotHandle>>>> = OnceLock::new();

pub(crate) fn slot_map() -> &'static Arc<DashMap<String, Arc<SlotHandle>>> {
    SLOT_MAP.get_or_init(|| Arc::new(DashMap::new()))
}

static CHILD_PIDS: OnceLock<DashSet<u32>> = OnceLock::new();
pub(crate) fn child_pids() -> &'static DashSet<u32> {
    CHILD_PIDS.get_or_init(DashSet::new)
}

pub(crate) fn register_child_pid(pid: u32) {
    child_pids().insert(pid);
}

pub(crate) fn unregister_child_pid(pid: u32) {
    child_pids().remove(&pid);
}

pub(crate) fn get_slot_handle(
    runtime_key: &str,
    role_name: &str,
    app_session_id: &str,
) -> Arc<SlotHandle> {
    let key = pool_key(app_session_id, runtime_key, role_name);
    slot_map()
        .entry(key)
        .or_insert_with(|| {
            Arc::new(SlotHandle {
                conn: tokio::sync::Mutex::new(None),
                prompt_lock: tokio::sync::Mutex::new(()),
            })
        })
        .clone()
}
