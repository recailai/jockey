//! Which tool calls of a connection are still running.
//!
//! claude-agent-acp delivers a steered message with the Claude SDK's `now` priority, which
//! pre-empts the running generation: a Bash command or a foreground sub-agent in flight is
//! cut off. "Send now" therefore waits until no tool call is running, so the message lands
//! between steps instead of stopping one.

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

#[cfg(test)]
mod tests {
    use super::*;

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
