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

pub fn respond_to_permission(
    request_id: &str,
    outcome: acp::RequestPermissionOutcome,
) -> Result<(), String> {
    if let acp::RequestPermissionOutcome::Selected(selected) = &outcome {
        let pending = permission_requests()
            .get(request_id)
            .ok_or("This approval has expired or was already handled.")?;
        let option_id = selected.option_id.to_string();
        if !pending
            .options
            .iter()
            .any(|option| option["optionId"].as_str() == Some(option_id.as_str()))
        {
            return Err("Unknown approval option.".into());
        }
    }
    let (_, pending) = permission_requests()
        .remove(request_id)
        .ok_or("This approval has expired or was already handled.")?;
    let remembered = match &outcome {
        acp::RequestPermissionOutcome::Selected(selected) => {
            let id = selected.option_id.to_string();
            pending.allow_always_option_ids.contains(&id).then_some(id)
        }
        _ => None,
    };
    let delivered = pending.tx.send(outcome).is_ok();
    crate::engine::events::emit(
        "permission_update",
        serde_json::json!({ "appSessionId": pending.app_session_id, "requestId": request_id }),
    );
    if !delivered {
        return Err("The agent is no longer waiting for this approval.".into());
    }
    if let Some(id) = remembered {
        if let Ok(mut cache) = approval_cache().lock() {
            cache.insert(pending.cache_key, id);
        }
    }
    Ok(())
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

#[cfg(test)]
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
pub fn respond_to_user_input(
    request_id: &str,
    answers: Option<UserInputAnswers>,
) -> Result<(), String> {
    let (_, pending) = user_input_requests()
        .remove(request_id)
        .ok_or("This question has expired or was already answered.")?;
    let delivered = pending.tx.send(answers).is_ok();
    crate::engine::events::emit(
        "permission_update",
        serde_json::json!({ "appSessionId": pending.info.app_session_id, "requestId": request_id }),
    );
    if delivered {
        Ok(())
    } else {
        Err("The agent is no longer waiting for this answer.".into())
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
    // Questions block the same session as approvals; cancelling one agent must
    // release its forms without dismissing questions from other sessions.
    let questions: Vec<String> = user_input_requests()
        .iter()
        .filter(|entry| {
            entry.info.app_session_id == app_session_id && entry.info.role_name == role_name
        })
        .map(|entry| entry.key().clone())
        .collect();
    for request_id in questions {
        let _ = respond_to_user_input(&request_id, None);
    }
    crate::engine::events::emit(
        "permission_update",
        serde_json::json!({ "appSessionId": app_session_id }),
    );
}

/// Drop every outstanding form. Called alongside permission cancellation so a session
/// reset does not leave the adapter parked on a question nobody will answer.
pub(crate) fn cancel_all_user_input() {
    let ids: Vec<String> = user_input_requests()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    for id in ids {
        let _ = respond_to_user_input(&id, None);
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
    fn cancelling_a_session_releases_only_its_questions() {
        let (a_tx, mut a_rx) = oneshot::channel();
        let (b_tx, mut b_rx) = oneshot::channel();
        for (id, sid, tx) in [
            ("cancel-question-a", "cancel-session-a", a_tx),
            ("cancel-question-b", "cancel-session-b", b_tx),
        ] {
            insert_user_input(
                PendingUserInputInfo {
                    request_id: id.into(),
                    role_name: "r".into(),
                    app_session_id: sid.into(),
                    title: None,
                    blocking: true,
                    questions: vec![],
                },
                tx,
            );
        }
        cancel_permissions_for("mock", "r", "cancel-session-a");
        assert_eq!(a_rx.try_recv().unwrap(), None);
        assert!(matches!(
            b_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        respond_to_user_input("cancel-question-b", None).unwrap();
    }

    #[test]
    fn decisions_reject_stale_options_and_disconnected_agents() {
        let (tx, rx) = oneshot::channel();
        insert_permission(
            "decision-contract".into(),
            PendingPermission {
                runtime_key: "mock".into(),
                role_name: "r".into(),
                app_session_id: "decision-test".into(),
                cache_key: "decision-contract-cache".into(),
                allow_always_option_ids: vec!["always".into()],
                delta_tx: None,
                tx,
                title: None,
                description: None,
                options: vec![serde_json::json!({ "optionId": "always" })],
            },
        );
        let selected = |id: &str| {
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                acp::PermissionOptionId::from(id.to_owned()),
            ))
        };
        assert!(respond_to_permission("decision-contract", selected("unknown")).is_err());
        assert!(permission_requests().contains_key("decision-contract"));
        drop(rx);
        assert!(respond_to_permission("decision-contract", selected("always")).is_err());
        assert!(cached_approval("decision-contract-cache").is_none());
        assert!(respond_to_permission(
            "decision-contract",
            acp::RequestPermissionOutcome::Cancelled
        )
        .is_err());
        assert!(respond_to_user_input("missing-question", None).is_err());
    }

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
        assert!(all
            .permissions
            .iter()
            .any(|p| p.request_id == "snap-perm-a"));
        assert!(all.user_inputs.iter().any(|u| u.request_id == "snap-ask-b"));

        respond_to_user_input("snap-ask-b", None).unwrap();
        respond_to_permission("snap-perm-a", acp::RequestPermissionOutcome::Cancelled).unwrap();
        assert!(list_pending_prompts(Some("snap-s2")).user_inputs.is_empty());
        assert!(list_pending_prompts(Some("snap-s1")).permissions.is_empty());
    }
}
