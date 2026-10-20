use crate::acp::protocol as acp;
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
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) options: Vec<serde_json::Value>,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermissionInfo {
    pub request_id: String,
    pub runtime_key: String,
    pub role_name: String,
    pub app_session_id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<serde_json::Value>,
}

pub fn list_pending_permissions() -> Vec<PendingPermissionInfo> {
    permission_requests()
        .iter()
        .map(|entry| {
            let (req_id, p) = entry.pair();
            PendingPermissionInfo {
                request_id: req_id.clone(),
                runtime_key: p.runtime_key.clone(),
                role_name: p.role_name.clone(),
                app_session_id: p.app_session_id.clone(),
                title: p
                    .title
                    .clone()
                    .unwrap_or_else(|| "Permission Request".to_string()),
                description: p.description.clone(),
                options: p.options.clone(),
            }
        })
        .collect()
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

/// Answers to a structured `UserInputRequest`, keyed by question id.
pub type UserInputAnswers = std::collections::HashMap<String, Vec<String>>;

/// What a late-joining client needs to re-render a pending question; mirrors the
/// `userInputRequest` stream event plus the session/role it belongs to.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PendingUserInputInfo {
    pub request_id: String,
    pub role_name: String,
    pub app_session_id: String,
    pub title: Option<String>,
    pub blocking: bool,
    pub questions: Vec<serde_json::Value>,
}

struct PendingUserInput {
    tx: oneshot::Sender<Option<UserInputAnswers>>,
    info: PendingUserInputInfo,
}

static USER_INPUT_REQUESTS: OnceLock<DashMap<String, PendingUserInput>> = OnceLock::new();

fn user_input_requests() -> &'static DashMap<String, PendingUserInput> {
    USER_INPUT_REQUESTS.get_or_init(DashMap::new)
}

pub(crate) fn insert_user_input(
    info: PendingUserInputInfo,
    tx: oneshot::Sender<Option<UserInputAnswers>>,
) {
    user_input_requests().insert(info.request_id.clone(), PendingUserInput { tx, info });
}

#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PendingPrompts {
    pub permissions: Vec<PendingPermissionInfo>,
    pub user_inputs: Vec<PendingUserInputInfo>,
}

/// Prompts the agent is currently blocked on. Stream events are fire-and-forget, so a client
/// that connects after one was emitted recovers it from here.
pub fn list_pending_prompts(app_session_id: Option<&str>) -> PendingPrompts {
    let wanted = |sid: &str| app_session_id.map_or(true, |want| want == sid);
    let mut permissions: Vec<_> = list_pending_permissions()
        .into_iter()
        .filter(|p| wanted(&p.app_session_id))
        .collect();
    let mut user_inputs: Vec<_> = user_input_requests()
        .iter()
        .map(|e| e.info.clone())
        .filter(|i| wanted(&i.app_session_id))
        .collect();
    permissions.sort_by(|a, b| a.request_id.cmp(&b.request_id));
    user_inputs.sort_by(|a, b| a.request_id.cmp(&b.request_id));
    PendingPrompts {
        permissions,
        user_inputs,
    }
}

/// `None` means the user dismissed the form; the adapter decides what that means for its
/// protocol (Codex treats it as no answers).
pub fn respond_to_user_input(request_id: &str, answers: Option<UserInputAnswers>) {
    if let Some((_, pending)) = user_input_requests().remove(request_id) {
        let _ = pending.tx.send(answers);
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

/// Drop every outstanding form. Called alongside permission cancellation so a session
/// reset does not leave the adapter parked on a question nobody will answer.
pub(crate) fn cancel_all_user_input() {
    let ids: Vec<String> = user_input_requests()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    for id in ids {
        respond_to_user_input(&id, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_prompts_snapshot_filters_by_session_and_clears_on_respond() {
        let (ptx, _prx) = oneshot::channel();
        insert_permission(
            "snap-perm-a".into(),
            PendingPermission {
                runtime_key: "codex-cli".into(),
                role_name: "r".into(),
                app_session_id: "snap-s1".into(),
                cache_key: String::new(),
                allow_always_option_ids: vec![],
                delta_tx: None,
                tx: ptx,
                title: Some("Run".into()),
                description: None,
                options: vec![],
            },
        );
        let (utx, _urx) = oneshot::channel();
        insert_user_input(
            PendingUserInputInfo {
                request_id: "snap-ask-b".into(),
                role_name: "r".into(),
                app_session_id: "snap-s2".into(),
                title: None,
                blocking: true,
                questions: vec![serde_json::json!({ "id": "q" })],
            },
            utx,
        );

        let s1 = list_pending_prompts(Some("snap-s1"));
        assert_eq!(s1.permissions.len(), 1);
        assert!(s1.user_inputs.is_empty());
        let s2 = list_pending_prompts(Some("snap-s2"));
        assert!(s2.permissions.is_empty());
        assert_eq!(s2.user_inputs[0].request_id, "snap-ask-b");
        let all = list_pending_prompts(None);
        assert!(all.permissions.iter().any(|p| p.request_id == "snap-perm-a"));
        assert!(all.user_inputs.iter().any(|u| u.request_id == "snap-ask-b"));

        respond_to_user_input("snap-ask-b", None);
        respond_to_permission(
            "snap-perm-a",
            acp::RequestPermissionOutcome::Cancelled,
        );
        assert!(list_pending_prompts(Some("snap-s2")).user_inputs.is_empty());
        assert!(list_pending_prompts(Some("snap-s1")).permissions.is_empty());
    }
}
