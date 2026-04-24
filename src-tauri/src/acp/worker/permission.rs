use agent_client_protocol as acp;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tokio::sync::{mpsc, oneshot};

use super::types::AcpEvent;

const APPROVAL_CACHE_LIMIT: usize = 256;

pub(crate) struct PendingPermission {
    pub(crate) runtime_key: String,
    pub(crate) role_name: String,
    pub(crate) app_session_id: String,
    pub(crate) cache_key: String,
    pub(crate) allow_always_option_ids: Vec<String>,
    pub(crate) delta_tx: Option<mpsc::Sender<AcpEvent>>,
    pub(crate) tx: oneshot::Sender<acp::RequestPermissionOutcome>,
}

static PERMISSION_REQUESTS: OnceLock<DashMap<String, PendingPermission>> = OnceLock::new();
static APPROVAL_CACHE: OnceLock<Mutex<ApprovalCache>> = OnceLock::new();

struct ApprovalCache {
    order: VecDeque<String>,
    values: std::collections::HashMap<String, String>,
}

impl ApprovalCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            values: std::collections::HashMap::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }

    fn insert(&mut self, key: String, value: String) {
        if !self.values.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.values.insert(key, value);
        while self.order.len() > APPROVAL_CACHE_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.values.remove(&old);
            }
        }
    }
}

pub(crate) fn permission_requests() -> &'static DashMap<String, PendingPermission> {
    PERMISSION_REQUESTS.get_or_init(DashMap::new)
}

fn approval_cache() -> &'static Mutex<ApprovalCache> {
    APPROVAL_CACHE.get_or_init(|| Mutex::new(ApprovalCache::new()))
}

pub(crate) fn cached_approval(cache_key: &str) -> Option<String> {
    approval_cache().lock().ok()?.get(cache_key)
}

pub(crate) fn insert_permission(request_id: String, pending: PendingPermission) {
    permission_requests().insert(request_id, pending);
}

pub fn respond_to_permission(request_id: &str, outcome: acp::RequestPermissionOutcome) {
    if let Some((_, pending)) = permission_requests().remove(request_id) {
        if let acp::RequestPermissionOutcome::Selected(selected) = &outcome {
            let option_id = selected.option_id.to_string();
            if pending
                .allow_always_option_ids
                .iter()
                .any(|id| id == &option_id)
            {
                if let Ok(mut cache) = approval_cache().lock() {
                    cache.insert(pending.cache_key.clone(), option_id);
                }
            }
        }
        let _ = pending.tx.send(outcome);
    }
}

pub(crate) fn cancel_permissions_for(runtime_key: &str, role_name: &str, app_session_id: &str) {
    let request_ids = permission_requests()
        .iter()
        .filter(|entry| {
            entry.runtime_key == runtime_key
                && entry.role_name == role_name
                && entry.app_session_id == app_session_id
        })
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for request_id in request_ids {
        if let Some((_, pending)) = permission_requests().remove(&request_id) {
            if let Some(delta_tx) = &pending.delta_tx {
                let _ = delta_tx.try_send(AcpEvent::PermissionExpired {
                    request_id: request_id.clone(),
                });
            }
            let _ = pending.tx.send(acp::RequestPermissionOutcome::Cancelled);
        }
    }
}

pub(crate) fn cancel_all_permissions() {
    let request_ids = permission_requests()
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    for request_id in request_ids {
        if let Some((_, pending)) = permission_requests().remove(&request_id) {
            if let Some(delta_tx) = &pending.delta_tx {
                let _ = delta_tx.try_send(AcpEvent::PermissionExpired {
                    request_id: request_id.clone(),
                });
            }
            let _ = pending.tx.send(acp::RequestPermissionOutcome::Cancelled);
        }
    }
}
