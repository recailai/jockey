//! ACP sub-agents (`unstable_subagents`) as tool rows of the parent turn.
//!
//! A child session announced by `subagent_update` becomes one synthetic tool call,
//! `subagent:<child id>`, of kind `agent`. The child's own traffic arrives on the same
//! connection under the child's session id: its text streams into that row's output and its
//! tool calls nest under the row through `parentId`. Reusing tool rows keeps live preview,
//! transcript checkpoints and history rendering on one path; a sub-agent never adds text to
//! the parent's reply.

use crate::acp::protocol as acp;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::worker::AcpEvent;

pub(crate) const ROW_PREFIX: &str = "subagent:";

#[derive(Default)]
struct Child {
    /// Row of the session that owns the child (`None` for the top-level session). Nested
    /// sub-agents nest their rows the same way their tools do.
    owner: Option<String>,
    description: Option<String>,
    prompt: Option<String>,
}

impl Child {
    /// The row's input. Sent whole: a tool update replaces `rawInput` rather than merging it.
    fn raw_input(&self) -> Option<Value> {
        if self.description.is_none() && self.prompt.is_none() {
            return None;
        }
        Some(json!({ "description": self.description, "prompt": self.prompt }))
    }
}

#[derive(Default)]
pub(crate) struct Subagents {
    children: HashMap<String, Child>,
    /// Message ids already shown. `session_message` is an upsert; a repeat is not new text.
    messages: HashSet<String>,
}

pub(crate) fn row_id(child: &str) -> String {
    format!("{ROW_PREFIX}{child}")
}

impl Subagents {
    pub(crate) fn is_child(&self, session: &str) -> bool {
        self.children.contains_key(session)
    }

    /// The row that a session's traffic belongs to: `None` for the top-level session.
    pub(crate) fn scope_of(&self, session: &str) -> Option<String> {
        self.is_child(session).then(|| row_id(session))
    }

    /// A `subagent_update` received on `scope` (the announcing session's row, if it is
    /// itself a child). The first update for a child opens its row; later ones patch it.
    pub(crate) fn update(
        &mut self,
        scope: Option<String>,
        update: acp::SubagentUpdate,
    ) -> AcpEvent {
        let child = update.session_id.to_string();
        let first = !self.children.contains_key(&child);
        let entry = self.children.entry(child.clone()).or_insert_with(|| Child {
            owner: scope,
            ..Child::default()
        });
        let title = match (&update.title, &update.description) {
            (acp::MaybeUndefined::Value(t), _) if !t.trim().is_empty() => Some(t.clone()),
            (_, acp::MaybeUndefined::Value(d)) if !d.trim().is_empty() => Some(d.clone()),
            _ => None,
        };
        let described = match &update.description {
            acp::MaybeUndefined::Value(d) if !d.trim().is_empty() => {
                entry.description = Some(d.clone());
                true
            }
            acp::MaybeUndefined::Null => {
                entry.description = None;
                true
            }
            _ => false,
        };
        let status = match &update.state {
            acp::MaybeUndefined::Value(state) => Some(status_of(state)),
            _ => None,
        };
        let raw_input = if first || described {
            entry.raw_input()
        } else {
            None
        };
        let owner = entry.owner.clone();
        let id = row_id(&child);
        if first {
            AcpEvent::ToolCall {
                tool_call_id: id,
                title: title.unwrap_or_else(|| "Sub-agent".to_string()),
                tool_name: Some("Agent".to_string()),
                tool_kind: "agent".to_string(),
                status: status.unwrap_or_else(|| "in_progress".to_string()),
                content: None,
                locations: None,
                raw_input,
                raw_output: None,
                terminal_meta: None,
                parent_id: owner,
                diff: None,
            }
        } else {
            AcpEvent::ToolCallUpdate {
                tool_call_id: id,
                tool_name: None,
                tool_kind: None,
                status,
                title,
                content: None,
                locations: None,
                raw_input,
                raw_output: None,
                terminal_meta: None,
                parent_id: None,
                diff: None,
            }
        }
    }

    /// An inter-session message. A message to a child is its task and becomes the row's
    /// input; a message from a child is its report and streams into the row's output.
    pub(crate) fn message(
        &mut self,
        message_id: &str,
        sender: Option<&acp::SessionId>,
        recipient: Option<&acp::SessionId>,
        text: String,
        chunk: bool,
    ) -> Option<AcpEvent> {
        if text.is_empty() || (!chunk && !self.messages.insert(message_id.to_string())) {
            return None;
        }
        let known =
            |s: Option<&acp::SessionId>| s.map(ToString::to_string).filter(|s| self.is_child(s));
        if let Some(child) = known(recipient) {
            let entry = self.children.get_mut(&child)?;
            entry.prompt = match (chunk, entry.prompt.take()) {
                (true, Some(prompt)) => Some(prompt + &text),
                _ => Some(text),
            };
            return Some(input_update(&child, entry.raw_input()));
        }
        let child = known(sender)?;
        Some(AcpEvent::ToolOutputDelta {
            tool_call_id: row_id(&child),
            delta: if chunk { text } else { format!("{text}\n\n") },
        })
    }

    /// Re-homes an event from a child session onto its row. Only what the row can show is
    /// kept: the child's thoughts, plans, usage and mode changes are its own business.
    pub(crate) fn scoped(row: &str, event: AcpEvent) -> Option<AcpEvent> {
        match event {
            AcpEvent::TextDelta { text } => Some(AcpEvent::ToolOutputDelta {
                tool_call_id: row.to_string(),
                delta: text,
            }),
            AcpEvent::ToolCall {
                tool_call_id,
                title,
                tool_name,
                tool_kind,
                status,
                content,
                locations,
                raw_input,
                raw_output,
                terminal_meta,
                parent_id,
                diff,
            } => Some(AcpEvent::ToolCall {
                tool_call_id,
                title,
                tool_name,
                tool_kind,
                status,
                content,
                locations,
                raw_input,
                raw_output,
                terminal_meta,
                parent_id: parent_id.or_else(|| Some(row.to_string())),
                diff,
            }),
            AcpEvent::ToolCallUpdate {
                tool_call_id,
                tool_name,
                tool_kind,
                status,
                title,
                content,
                locations,
                raw_input,
                raw_output,
                terminal_meta,
                parent_id,
                diff,
            } => Some(AcpEvent::ToolCallUpdate {
                tool_call_id,
                tool_name,
                tool_kind,
                status,
                title,
                content,
                locations,
                raw_input,
                raw_output,
                terminal_meta,
                parent_id: parent_id.or_else(|| Some(row.to_string())),
                diff,
            }),
            AcpEvent::ToolOutputDelta { .. } => Some(event),
            _ => None,
        }
    }
}

fn input_update(child: &str, raw_input: Option<Value>) -> AcpEvent {
    AcpEvent::ToolCallUpdate {
        tool_call_id: row_id(child),
        tool_name: None,
        tool_kind: None,
        status: None,
        title: None,
        content: None,
        locations: None,
        raw_input,
        raw_output: None,
        terminal_meta: None,
        parent_id: None,
        diff: None,
    }
}

/// Tool-call status for a child's work state. `idle` ends a run: it failed when it stopped on
/// an error, otherwise it finished. `requires_action` is waiting on the user, like a pending
/// permission.
fn status_of(state: &acp::StateUpdate) -> String {
    let value = serde_json::to_value(state).unwrap_or(Value::Null);
    let field = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or("");
    match field("state") {
        "running" => "in_progress",
        "requires_action" => "pending",
        "idle" => match field("stopReason") {
            "error" | "refusal" => "failed",
            "cancelled" => "cancelled",
            _ => "completed",
        },
        _ => "in_progress",
    }
    .to_string()
}

/// The text of content blocks; non-text blocks (images, resources) are not shown in a row.
pub(crate) fn text_of<'a>(blocks: impl IntoIterator<Item = &'a acp::ContentBlock>) -> String {
    blocks
        .into_iter()
        .filter_map(|b| match b {
            acp::ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(raw: Value) -> acp::SubagentUpdate {
        serde_json::from_value(raw).unwrap()
    }

    #[test]
    fn first_update_opens_a_row_and_later_ones_patch_it() {
        let mut subagents = Subagents::default();
        let opened = subagents.update(
            None,
            update(json!({
                "sessionId": "c1",
                "title": "explorer (Explore)",
                "description": "Find the parser",
                "capabilities": { "cancel": {} },
                "state": { "state": "running" }
            })),
        );
        match opened {
            AcpEvent::ToolCall {
                tool_call_id,
                title,
                tool_kind,
                status,
                parent_id,
                raw_input,
                ..
            } => {
                assert_eq!(tool_call_id, "subagent:c1");
                assert_eq!(title, "explorer (Explore)");
                assert_eq!(tool_kind, "agent");
                assert_eq!(status, "in_progress");
                assert_eq!(parent_id, None);
                assert_eq!(
                    raw_input,
                    Some(json!({ "description": "Find the parser", "prompt": null }))
                );
            }
            _ => panic!("expected a tool call"),
        }
        assert_eq!(subagents.scope_of("c1").as_deref(), Some("subagent:c1"));
        let done = subagents.update(
            None,
            update(json!({ "sessionId": "c1", "state": { "state": "idle", "stopReason": "end_turn" } })),
        );
        assert!(matches!(
            done,
            AcpEvent::ToolCallUpdate { status: Some(ref s), .. } if s == "completed"
        ));
        let failed = subagents.update(
            None,
            update(
                json!({ "sessionId": "c1", "state": { "state": "idle", "stopReason": "error" } }),
            ),
        );
        assert!(matches!(
            failed,
            AcpEvent::ToolCallUpdate { status: Some(ref s), .. } if s == "failed"
        ));
    }

    #[test]
    fn nested_children_hang_off_their_owner_row() {
        let mut subagents = Subagents::default();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        let nested = subagents.update(
            subagents.scope_of("c1"),
            update(json!({ "sessionId": "c2" })),
        );
        assert!(matches!(
            nested,
            AcpEvent::ToolCall { parent_id: Some(ref p), .. } if p == "subagent:c1"
        ));
    }

    #[test]
    fn child_traffic_is_scoped_to_its_row() {
        let text = Subagents::scoped("subagent:c1", AcpEvent::TextDelta { text: "hi".into() });
        assert!(matches!(
            text,
            Some(AcpEvent::ToolOutputDelta { ref tool_call_id, ref delta })
                if tool_call_id == "subagent:c1" && delta == "hi"
        ));
        let thought = Subagents::scoped("subagent:c1", AcpEvent::ThoughtDelta { text: "x".into() });
        assert!(thought.is_none());
    }

    #[test]
    fn messages_become_task_input_or_report_output_once() {
        let mut subagents = Subagents::default();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        let parent = acp::SessionId::new("p");
        let child = acp::SessionId::new("c1");
        let task = subagents.message("m1", Some(&parent), Some(&child), "Find it".into(), false);
        assert!(matches!(
            task,
            Some(AcpEvent::ToolCallUpdate { raw_input: Some(ref v), .. }) if v["prompt"] == "Find it"
        ));
        assert!(subagents
            .message("m1", Some(&parent), Some(&child), "Find it".into(), false)
            .is_none());
        let report = subagents.message("m2", Some(&child), Some(&parent), "Found".into(), false);
        assert!(matches!(
            report,
            Some(AcpEvent::ToolOutputDelta { ref delta, .. }) if delta == "Found\n\n"
        ));
        assert!(subagents
            .message("m3", Some(&parent), None, "elsewhere".into(), false)
            .is_none());
    }
}
