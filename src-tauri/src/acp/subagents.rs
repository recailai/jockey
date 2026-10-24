//! ACP sub-agents (`unstable_subagents`) as tool rows of the parent turn.
//!
//! A child session announced by `subagent_update` becomes one synthetic tool call,
//! `subagent:<child id>`, of kind `agent`. The child's own traffic arrives on the same
//! connection under the child's session id: its text streams into that row's output and its
//! tool calls nest under the row through `parentId`. Reusing tool rows keeps live preview,
//! transcript checkpoints and history rendering on one path; a sub-agent never adds text to
//! the parent's reply.
//!
//! Rows belong to one turn. The connection outlives turns, so each child carries the turn
//! generation it was announced in: a prompt's end settles the rows still open and makes every
//! child stale, and a stale child's traffic is dropped until it announces itself again (which
//! reopens its row in the new turn). Without this, old children would patch rows that the new
//! turn's transcript never saw.

use crate::acp::protocol as acp;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::client::RAW_PAYLOAD_LIMIT;
use super::worker::AcpEvent;

pub(crate) const ROW_PREFIX: &str = "subagent:";

#[derive(Default)]
struct Child {
    /// Row of the session that owns the child (`None` for the top-level session). Nested
    /// sub-agents nest their rows the same way their tools do.
    owner: Option<String>,
    description: Option<String>,
    prompt: Option<String>,
    /// The inter-session message `prompt` was built from; chunks of another message replace
    /// it instead of extending it.
    message_id: Option<String>,
    generation: u64,
    /// The row has not reached a terminal status yet.
    open: bool,
    /// Permission requests the child raised, so cancelling it can expire them.
    permissions: Vec<String>,
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
    generation: u64,
}

pub(crate) fn row_id(child: &str) -> String {
    format!("{ROW_PREFIX}{child}")
}

/// The child session id of a sub-agent row id.
pub(crate) fn child_of_row(id: &str) -> Option<&str> {
    id.strip_prefix(ROW_PREFIX)
}

/// The fields a `subagent_update` can change on an existing row.
#[derive(Default)]
struct RowPatch {
    status: Option<String>,
    title: Option<String>,
    raw_input: Option<Value>,
}

fn row_update(child: &str, patch: RowPatch) -> AcpEvent {
    AcpEvent::ToolCallUpdate {
        tool_call_id: row_id(child),
        tool_name: None,
        tool_kind: None,
        status: patch.status,
        title: patch.title,
        content: None,
        locations: None,
        raw_input: patch.raw_input,
        raw_output: None,
        terminal_meta: None,
        parent_id: None,
        diff: None,
    }
}

fn row_open(child: &str, owner: Option<String>, title: String, patch: RowPatch) -> AcpEvent {
    AcpEvent::ToolCall {
        tool_call_id: row_id(child),
        title,
        tool_name: Some("Agent".to_string()),
        tool_kind: "agent".to_string(),
        status: patch.status.unwrap_or_else(|| "in_progress".to_string()),
        content: None,
        locations: None,
        raw_input: patch.raw_input,
        raw_output: None,
        terminal_meta: None,
        parent_id: owner,
        diff: None,
    }
}

/// Appends `text` without letting `target` grow past the shared payload cap.
fn push_capped(target: &mut String, text: &str) {
    let room = RAW_PAYLOAD_LIMIT.saturating_sub(target.len());
    let mut end = text.len().min(room);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&text[..end]);
}

impl Subagents {
    pub(crate) fn is_child(&self, session: &str) -> bool {
        self.children.contains_key(session)
    }

    /// Announced in an earlier turn: its traffic no longer belongs to any row.
    pub(crate) fn is_stale(&self, session: &str) -> bool {
        self.children
            .get(session)
            .is_some_and(|c| c.generation != self.generation)
    }

    /// The row that a session's traffic belongs to: `None` for the top-level session.
    pub(crate) fn scope_of(&self, session: &str) -> Option<String> {
        self.is_child(session).then(|| row_id(session))
    }

    /// A new prompt starts: whatever was announced before is stale.
    pub(crate) fn begin_turn(&mut self) {
        self.generation += 1;
        self.messages.clear();
    }

    /// The prompt ended. Closes the rows still open with `status` (the turn's outcome), since
    /// a child's own final `idle` can no longer reach the turn, and makes every child stale.
    pub(crate) fn end_turn(&mut self, status: &str) -> Vec<AcpEvent> {
        let generation = self.generation;
        let mut open: Vec<&String> = self
            .children
            .iter()
            .filter(|(_, c)| c.generation == generation && c.open)
            .map(|(id, _)| id)
            .collect();
        open.sort();
        let events = open
            .into_iter()
            .map(|id| {
                row_update(
                    id,
                    RowPatch {
                        status: Some(status.to_string()),
                        ..RowPatch::default()
                    },
                )
            })
            .collect();
        for child in self.children.values_mut() {
            child.open = false;
        }
        self.generation += 1;
        events
    }

    /// Remembers a permission request raised by `child`.
    pub(crate) fn track_permission(&mut self, child: &str, request_id: &str) {
        if let Some(child) = self.children.get_mut(child) {
            child.permissions.push(request_id.to_string());
        }
    }

    /// Hands over (and forgets) the permission requests `child` raised.
    pub(crate) fn take_permissions(&mut self, child: &str) -> Vec<String> {
        self.children
            .get_mut(child)
            .map(|c| std::mem::take(&mut c.permissions))
            .unwrap_or_default()
    }

    /// A `subagent_update` received on `scope` (the announcing session's row, if it is
    /// itself a child). The first update for a child in a turn opens its row; later ones
    /// patch it.
    pub(crate) fn update(
        &mut self,
        scope: Option<String>,
        update: acp::SubagentUpdate,
    ) -> AcpEvent {
        let child = update.session_id.to_string();
        let generation = self.generation;
        let first = self
            .children
            .get(&child)
            .is_none_or(|c| c.generation != generation);
        if first {
            self.children.insert(
                child.clone(),
                Child {
                    owner: scope,
                    generation,
                    open: true,
                    ..Child::default()
                },
            );
        }
        let entry = self.children.get_mut(&child).expect("inserted above");
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
        if let Some(status) = &status {
            entry.open = !is_terminal(status);
        }
        let patch = RowPatch {
            status,
            raw_input: (first || described).then(|| entry.raw_input()).flatten(),
            title: title.clone(),
        };
        if first {
            let owner = entry.owner.clone();
            row_open(
                &child,
                owner,
                title.unwrap_or_else(|| "Sub-agent".to_string()),
                RowPatch {
                    title: None,
                    ..patch
                },
            )
        } else {
            row_update(&child, patch)
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
            let mut prompt = match entry.prompt.take() {
                Some(prompt) if chunk && entry.message_id.as_deref() == Some(message_id) => prompt,
                _ => String::new(),
            };
            push_capped(&mut prompt, &text);
            entry.message_id = Some(message_id.to_string());
            entry.prompt = Some(prompt);
            return Some(row_update(
                &child,
                RowPatch {
                    raw_input: entry.raw_input(),
                    ..RowPatch::default()
                },
            ));
        }
        let child = known(sender)?;
        Some(AcpEvent::ToolOutputDelta {
            tool_call_id: row_id(&child),
            delta: if chunk { text } else { format!("{text}\n\n") },
        })
    }

    /// Re-homes an event from a child session onto its row. Only what the row can show is
    /// kept: the child's thoughts, plans, usage and mode changes are its own business.
    pub(crate) fn scoped(row: &str, mut event: AcpEvent) -> Option<AcpEvent> {
        match &mut event {
            AcpEvent::TextDelta { text } => {
                return Some(AcpEvent::ToolOutputDelta {
                    tool_call_id: row.to_string(),
                    delta: std::mem::take(text),
                })
            }
            AcpEvent::ToolCall { parent_id, .. } | AcpEvent::ToolCallUpdate { parent_id, .. } => {
                parent_id.get_or_insert_with(|| row.to_string());
            }
            AcpEvent::ToolOutputDelta { .. } => {}
            _ => return None,
        }
        Some(event)
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
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

    #[test]
    fn description_null_clears_the_input_and_requires_action_is_pending() {
        let mut subagents = Subagents::default();
        subagents.update(
            None,
            update(json!({ "sessionId": "c1", "description": "Find it" })),
        );
        let cleared = subagents.update(
            None,
            update(json!({
                "sessionId": "c1",
                "description": null,
                "state": { "state": "requires_action" }
            })),
        );
        match cleared {
            AcpEvent::ToolCallUpdate {
                status, raw_input, ..
            } => {
                assert_eq!(status.as_deref(), Some("pending"));
                assert_eq!(
                    raw_input, None,
                    "no description and no prompt: nothing to show"
                );
            }
            _ => panic!("expected an update"),
        }
    }

    #[test]
    fn a_new_message_replaces_the_prompt_and_chunks_extend_it() {
        let mut subagents = Subagents::default();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        let (parent, child) = (acp::SessionId::new("p"), acp::SessionId::new("c1"));
        let prompt = |e: Option<AcpEvent>| match e {
            Some(AcpEvent::ToolCallUpdate {
                raw_input: Some(v), ..
            }) => v["prompt"].as_str().unwrap().to_string(),
            _ => panic!("expected an input update"),
        };
        let one = subagents.message("m1", Some(&parent), Some(&child), "ab".into(), true);
        assert_eq!(prompt(one), "ab");
        let more = subagents.message("m1", Some(&parent), Some(&child), "cd".into(), true);
        assert_eq!(prompt(more), "abcd");
        let next = subagents.message("m2", Some(&parent), Some(&child), "xy".into(), true);
        assert_eq!(prompt(next), "xy");
    }

    #[test]
    fn the_prompt_is_capped_on_a_char_boundary() {
        let mut subagents = Subagents::default();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        let (parent, child) = (acp::SessionId::new("p"), acp::SessionId::new("c1"));
        let big = "é".repeat(RAW_PAYLOAD_LIMIT);
        let event = subagents.message("m1", Some(&parent), Some(&child), big, true);
        let Some(AcpEvent::ToolCallUpdate {
            raw_input: Some(v), ..
        }) = event
        else {
            panic!("expected an input update");
        };
        let len = v["prompt"].as_str().unwrap().len();
        assert!(len <= RAW_PAYLOAD_LIMIT && len > RAW_PAYLOAD_LIMIT - 2);
    }

    #[test]
    fn stale_children_are_dropped_until_they_announce_again() {
        let mut subagents = Subagents::default();
        subagents.begin_turn();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        assert!(!subagents.is_stale("c1"));
        subagents.begin_turn();
        assert!(subagents.is_stale("c1"));
        assert!(
            subagents.is_child("c1"),
            "still known, so its requests validate"
        );
        let reopened = subagents.update(None, update(json!({ "sessionId": "c1" })));
        assert!(matches!(reopened, AcpEvent::ToolCall { .. }));
        assert!(!subagents.is_stale("c1"));
    }

    #[test]
    fn ending_a_turn_settles_open_rows_once_and_ignores_finished_ones() {
        let mut subagents = Subagents::default();
        subagents.begin_turn();
        subagents.update(None, update(json!({ "sessionId": "b" })));
        subagents.update(None, update(json!({ "sessionId": "a" })));
        subagents.update(
            None,
            update(json!({ "sessionId": "done", "state": { "state": "idle", "stopReason": "end_turn" } })),
        );
        let settled = subagents.end_turn("cancelled");
        let ids: Vec<_> = settled
            .iter()
            .map(|e| match e {
                AcpEvent::ToolCallUpdate {
                    tool_call_id,
                    status: Some(status),
                    ..
                } => (tool_call_id.as_str(), status.as_str()),
                _ => panic!("expected updates"),
            })
            .collect();
        assert_eq!(
            ids,
            vec![("subagent:a", "cancelled"), ("subagent:b", "cancelled")]
        );
        assert!(subagents.end_turn("completed").is_empty());
        assert!(subagents.is_stale("a"));
    }

    #[test]
    fn permissions_are_tracked_per_child_and_taken_once() {
        let mut subagents = Subagents::default();
        subagents.update(None, update(json!({ "sessionId": "c1" })));
        subagents.track_permission("c1", "r1");
        subagents.track_permission("nope", "r2");
        assert_eq!(subagents.take_permissions("c1"), vec!["r1".to_string()]);
        assert!(subagents.take_permissions("c1").is_empty());
    }

    #[test]
    fn row_ids_round_trip() {
        assert_eq!(child_of_row(&row_id("c1")), Some("c1"));
        assert_eq!(child_of_row("toolu_1"), None);
    }
}
