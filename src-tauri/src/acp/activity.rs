//! Steering into a running turn without cutting off the step in flight.
//!
//! claude-agent-acp delivers a steered message with the Claude SDK's `now` priority, which
//! pre-empts the running generation: a Bash command or a foreground sub-agent in flight is
//! cut off (claude-agent-acp#1341). Every steer asks for `next` delivery — the SDK then
//! inserts the message at the next step boundary — under both spellings proposed upstream
//! (`delivery`, #1175; `priority`, #1350); adapters ignore `_meta.steering` keys they do not
//! know. Until an adapter advertises that it honours it, "Send now" also waits here until no
//! tool call is running, so the message lands between steps either way.

use serde_json::{json, Value};
use std::collections::HashSet;

use super::worker::AcpEvent;

#[derive(Default)]
pub(crate) struct ToolActivity {
    open: HashSet<String>,
}

fn is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "canceled" | "error"
    )
}

impl ToolActivity {
    pub(crate) fn observe(&mut self, event: &AcpEvent) {
        match event {
            AcpEvent::ToolCall {
                tool_call_id,
                status,
                ..
            } => {
                if is_terminal(status) {
                    self.open.remove(tool_call_id);
                } else {
                    self.open.insert(tool_call_id.clone());
                }
            }
            AcpEvent::ToolCallUpdate {
                tool_call_id,
                status: Some(status),
                ..
            } if is_terminal(status) => {
                self.open.remove(tool_call_id);
            }
            _ => {}
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.open.is_empty()
    }

    /// A new turn starts clean; a call its predecessor never closed is not still running.
    pub(crate) fn clear(&mut self) {
        self.open.clear();
    }
}

/// Whether `initialize`'s `_meta.steering` says a `next` delivery is honoured, as proposed in
/// claude-agent-acp#1341 (`delivery: ["next"]`) or alongside #1350 (`priority: ["next"]`).
pub(crate) fn advertises_next_delivery(steering: &Value) -> bool {
    ["delivery", "priority"].iter().any(|key| {
        steering
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|v| v.as_str() == Some("next")))
    })
}

/// `_session/steering` params. `promptRequired`: if the turn ended meanwhile the adapter hands
/// the input back instead of starting a detached turn the engine would not own.
pub(crate) fn steer_params(session_id: &str, prompt: &Value) -> Value {
    json!({
        "sessionId": session_id,
        "prompt": prompt,
        "_meta": {
            "steering": {
                "idleBehavior": "promptRequired",
                "delivery": "next",
                "priority": "next",
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_delivery_is_read_from_either_proposed_spelling() {
        assert!(!advertises_next_delivery(&json!({ "supported": true })));
        assert!(advertises_next_delivery(
            &json!({ "supported": true, "delivery": ["next"] })
        ));
        assert!(advertises_next_delivery(
            &json!({ "priority": ["now", "next", "later"] })
        ));
        assert!(!advertises_next_delivery(&json!({ "delivery": "next" })));
    }

    #[test]
    fn steer_asks_for_next_delivery_and_a_host_owned_idle_fallback() {
        let params = steer_params("s1", &json!([{ "type": "text", "text": "hi" }]));
        assert_eq!(params["sessionId"], "s1");
        assert_eq!(
            params["_meta"]["steering"]["idleBehavior"],
            "promptRequired"
        );
        assert_eq!(params["_meta"]["steering"]["delivery"], "next");
        assert_eq!(params["_meta"]["steering"]["priority"], "next");
    }

    fn call(id: &str, status: &str) -> AcpEvent {
        AcpEvent::ToolCall {
            tool_call_id: id.into(),
            title: String::new(),
            tool_name: None,
            tool_kind: "execute".into(),
            status: status.into(),
            content: None,
            locations: None,
            raw_input: None,
            raw_output: None,
            terminal_meta: None,
            parent_id: None,
            diff: None,
        }
    }

    fn update(id: &str, status: &str) -> AcpEvent {
        AcpEvent::ToolCallUpdate {
            tool_call_id: id.into(),
            tool_name: None,
            tool_kind: None,
            status: Some(status.into()),
            title: None,
            content: None,
            locations: None,
            raw_input: None,
            raw_output: None,
            terminal_meta: None,
            parent_id: None,
            diff: None,
        }
    }

    #[test]
    fn idle_only_when_every_call_has_finished() {
        let mut activity = ToolActivity::default();
        assert!(activity.is_idle());
        activity.observe(&call("a", "pending"));
        activity.observe(&call("b", "in_progress"));
        assert!(!activity.is_idle());
        activity.observe(&update("a", "completed"));
        assert!(!activity.is_idle());
        activity.observe(&update("b", "in_progress"));
        assert!(!activity.is_idle());
        activity.observe(&update("b", "failed"));
        assert!(activity.is_idle());
    }

    #[test]
    fn clear_forgets_calls_left_open() {
        let mut activity = ToolActivity::default();
        activity.observe(&call("a", "pending"));
        activity.clear();
        assert!(activity.is_idle());
    }
}
