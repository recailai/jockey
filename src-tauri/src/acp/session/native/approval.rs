//! Bridge Codex's server→client approval requests into the Jockey permission UI.
//!
//! Before this, `NativeProcess::answer_server_request` blanket-accepted or blanket-declined
//! every approval, so Codex approvals never reached the user. These are blocking JSON-RPC
//! requests: Codex waits for the response, so it is correct to park the read loop while the
//! user decides.

use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::super::super::protocol as acp;
use super::super::super::worker::{
    insert_permission, insert_user_input, AcpEvent, PendingPermission, UserInputAnswers,
};
use super::NativeEventSink;

/// Methods that ask the user to allow or deny something.
pub(super) const APPROVAL_METHODS: &[&str] = &[
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/permissions/requestApproval",
    "applyPatchApproval",
    "execCommandApproval",
];

pub(super) fn is_approval_method(method: &str) -> bool {
    APPROVAL_METHODS.contains(&method)
}

/// Decision to send when the run is unattended (auto-approve) and no user can be asked.
pub(super) fn auto_decision(method: &str) -> &'static str {
    decisions_for(method).0
}

/// Codex spells the accepted decision differently per request family; the legacy
/// `applyPatchApproval` / `execCommandApproval` pair uses `ReviewDecision` values while the
/// `item/*` family uses `CommandExecutionApprovalDecision`.
fn decisions_for(method: &str) -> (&'static str, &'static str, &'static str) {
    match method {
        "applyPatchApproval" | "execCommandApproval" => {
            ("approved", "approvedForSession", "denied")
        }
        _ => ("accept", "acceptForSession", "decline"),
    }
}

fn describe(method: &str, params: &Value) -> (String, Option<String>) {
    let detail = params
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            params
                .get("reason")
                .or_else(|| params.get("explanation"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let title = match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => "Run command",
        "item/fileChange/requestApproval" | "applyPatchApproval" => "Apply file changes",
        "item/permissions/requestApproval" => "Grant permission",
        _ => "Approval required",
    };
    (title.to_string(), detail)
}

/// Emit the request to the UI and wait for the user. Returns the JSON-RPC `result` payload.
pub(super) async fn resolve_approval(
    sink: &mut NativeEventSink<'_>,
    method: &str,
    params: &Value,
    request_key: String,
) -> Value {
    let (accept, accept_session, decline) = decisions_for(method);
    let (title, description) = describe(method, params);

    let options = vec![
        json!({ "optionId": accept, "title": "Allow once", "kind": "allow_once" }),
        json!({ "optionId": accept_session, "title": "Allow for this session", "kind": "allow_always" }),
        json!({ "optionId": decline, "title": "Deny", "kind": "reject_once" }),
    ];

    let (runtime, role, app_session_id) = {
        let (r, ro, a) = sink.identity();
        (r.to_string(), ro.to_string(), a.to_string())
    };

    let (tx, rx) = oneshot::channel();
    insert_permission(
        request_key.clone(),
        PendingPermission {
            runtime_key: runtime,
            role_name: role,
            app_session_id,
            cache_key: format!("{method}:{}", description.as_deref().unwrap_or("")),
            allow_always_option_ids: vec![accept_session.to_string()],
            delta_tx: None,
            tx,
        },
    );

    sink.emit(AcpEvent::PermissionRequest {
        request_id: request_key,
        title,
        description,
        options,
    });

    // Denial is the safe default for a cancelled or dropped request: never run something the
    // user did not agree to.
    let decision = match rx.await {
        Ok(acp::RequestPermissionOutcome::Selected(selected)) => selected.option_id.to_string(),
        _ => decline.to_string(),
    };
    json!({ "decision": decision })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_item_families_use_different_decision_vocabularies() {
        // Sending "accept" to applyPatchApproval would be rejected by Codex: that family
        // expects ReviewDecision values.
        assert_eq!(decisions_for("applyPatchApproval").0, "approved");
        assert_eq!(decisions_for("item/fileChange/requestApproval").0, "accept");
        assert_eq!(decisions_for("execCommandApproval").2, "denied");
        assert_eq!(
            decisions_for("item/permissions/requestApproval").2,
            "decline"
        );
    }

    #[test]
    fn recognises_every_approval_method_from_the_generated_schema() {
        for method in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/permissions/requestApproval",
            "applyPatchApproval",
            "execCommandApproval",
        ] {
            assert!(is_approval_method(method), "{method} should be an approval");
        }
        assert!(!is_approval_method("item/tool/call"));
        assert!(!is_approval_method("mcpServer/elicitation/request"));
    }

    #[test]
    fn describe_surfaces_the_command_being_approved() {
        let (title, detail) = describe(
            "item/commandExecution/requestApproval",
            &json!({ "command": "rm -rf build" }),
        );
        assert_eq!(title, "Run command");
        assert_eq!(detail.as_deref(), Some("rm -rf build"));
    }
}

/// Codex's structured question surface. Unlike an approval this is not allow/deny: each
/// question may offer options, accept free text (`isOther`), or be a secret, and several
/// questions can arrive in one request.
pub(super) const USER_INPUT_METHOD: &str = "item/tool/requestUserInput";

pub(super) async fn resolve_user_input(
    sink: &mut NativeEventSink<'_>,
    params: &Value,
    request_key: String,
) -> Value {
    let questions = params
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let normalized: Vec<Value> = questions
        .iter()
        .map(|q| {
            json!({
                "id": q.get("id").and_then(Value::as_str).unwrap_or_default(),
                "header": q.get("header").and_then(Value::as_str),
                "prompt": q.get("question").and_then(Value::as_str).unwrap_or_default(),
                "options": q
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|opts| {
                        opts.iter()
                            .map(|o| {
                                json!({
                                    "value": o.get("label").and_then(Value::as_str).unwrap_or_default(),
                                    "label": o.get("label").and_then(Value::as_str).unwrap_or_default(),
                                    "description": o.get("description").and_then(Value::as_str),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                "allowOther": q.get("isOther").and_then(Value::as_bool).unwrap_or(false),
                "secret": q.get("isSecret").and_then(Value::as_bool).unwrap_or(false),
                "multi": false,
            })
        })
        .collect();

    let (tx, rx) = oneshot::channel();
    insert_user_input(request_key.clone(), tx);
    sink.emit(AcpEvent::UserInputRequest {
        request_id: request_key,
        title: None,
        blocking: params
            .get("isBlocking")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        questions: normalized,
    });

    // A dismissed or dropped form answers nothing rather than inventing a choice.
    let answers: UserInputAnswers = rx.await.ok().flatten().unwrap_or_default();
    json!({
        "answers": answers
            .into_iter()
            .map(|(id, values)| (id, json!({ "answers": values })))
            .collect::<serde_json::Map<_, _>>(),
    })
}
