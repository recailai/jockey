//! Turn transcript reducer: folds the live `acp/delta` + `acp/stream` frames of one turn into
//! the `AppMessage` shape every client renders and the database stores.
//!
//! This is the record-keeping half of the desktop's `acpEventBridge.ts` /
//! `finalizeSessionStream`, moved here so a turn is recorded identically no matter which
//! client started it. UI-only state (agent status line, plan, usage, permissions, modes,
//! commands) is deliberately not tracked: none of it was ever part of the persisted message.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

const MAX_THOUGHT_CHARS: usize = 5000;
const OUTPUT_LOG_LIMIT: usize = 256 * 1024;
const OUTPUT_LOG_KEEP: usize = 128 * 1024;

/// Event kinds that only drive live UI state and never become part of the transcript.
const UI_ONLY_KINDS: &[&str] = &[
    "statusUpdate",
    "userInputRequest",
    "contextCompacted",
    "usage",
    "notice",
    "plan",
    "permissionRequest",
    "permissionExpired",
    "modeUpdate",
    "availableModes",
    "availableCommands",
    "sessionError",
];

pub(crate) fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalize_tool_status(status: &str) -> String {
    let normalized: String = status
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect::<String>()
        .to_lowercase();
    match normalized.as_str() {
        "inprogress" | "running" => "running",
        "pending" => "pending",
        "completed" | "success" => "completed",
        "failed" | "failure" | "error" => "failure",
        "declined" | "cancelled" | "canceled" => "cancelled",
        "interrupted" => "interrupted",
        _ => return status.to_string(),
    }
    .to_string()
}

fn normalize_locations(raw: Option<&Value>) -> Option<Value> {
    let items = raw?.as_array()?;
    let out: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let path = item.get("path")?.as_str().filter(|p| !p.is_empty())?;
            let mut loc = json!({ "path": path });
            if let Some(line) = item.get("line").filter(|l| l.is_number()) {
                loc["line"] = line.clone();
            }
            Some(loc)
        })
        .collect();
    (!out.is_empty()).then(|| Value::Array(out))
}

fn non_null(v: Option<&Value>) -> Option<Value> {
    v.filter(|v| !v.is_null()).cloned()
}

fn role_of(seg: &Value) -> Option<&str> {
    seg.get("roleName").and_then(Value::as_str)
}

fn segment(kind: &str, role: Option<&str>) -> Map<String, Value> {
    let mut seg = Map::new();
    seg.insert("kind".into(), json!(kind));
    if let Some(role) = role {
        seg.insert("roleName".into(), json!(role));
    }
    seg
}

fn tool_segment(tc: Value, role: Option<&str>) -> Value {
    let mut seg = segment("tool", role);
    seg.insert("tc".into(), tc);
    Value::Object(seg)
}

fn is_tool_segment_for(seg: &Value, id: &str) -> bool {
    seg.get("kind").and_then(Value::as_str) == Some("tool")
        && seg.pointer("/tc/toolCallId").and_then(Value::as_str) == Some(id)
}

/// Drops the render-only `*Json` mirrors and bounds `outputLog`, matching what the desktop
/// has always written so old and new rows load the same way.
pub(crate) fn sanitize_tool_call(tc: &Value) -> Value {
    let mut tc = tc.clone();
    if let Value::Object(map) = &mut tc {
        map.remove("rawInputJson");
        map.remove("rawOutputJson");
        map.remove("contentJson");
        if let Some(Value::String(log)) = map.get_mut("outputLog") {
            if log.len() > OUTPUT_LOG_LIMIT {
                let head_end = floor_char_boundary(log, OUTPUT_LOG_KEEP);
                let tail_start = ceil_char_boundary(log, log.len() - OUTPUT_LOG_KEEP);
                *log = format!(
                    "{}\n...[truncated]...\n{}",
                    &log[..head_end],
                    &log[tail_start..]
                );
            }
        }
    }
    tc
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// How a turn ended, as far as the transcript cares.
pub(crate) enum TurnOutcome<'a> {
    Reply {
        ok: bool,
        reply: &'a str,
        role_replies: &'a [crate::types::RoleReply],
    },
    /// The turn errored before producing a response.
    Failed,
}

pub(crate) struct Transcript {
    fallback_role: String,
    /// Role shown on the single-role message. Starts as the route label and follows the
    /// role reported by text deltas, like the desktop's live `streamingMessage.roleName`.
    stream_role: String,
    text: String,
    segments: Vec<Value>,
    tool_order: Vec<String>,
    tools: HashMap<String, Value>,
    thought: String,
}

impl Transcript {
    pub(crate) fn new(role_label: &str) -> Self {
        Self {
            fallback_role: role_label.to_string(),
            stream_role: role_label.to_string(),
            text: String::new(),
            segments: Vec::new(),
            tool_order: Vec::new(),
            tools: HashMap::new(),
            thought: String::new(),
        }
    }

    pub(crate) fn has_content(&self) -> bool {
        !self.text.trim().is_empty() || !self.tools.is_empty() || !self.segments.is_empty()
    }

    /// One `acp/delta` batch. Returns whether the transcript changed.
    pub(crate) fn apply_text(&mut self, delta: &str, role: Option<&str>) -> bool {
        let normalized = normalize_newlines(delta);
        if normalized.is_empty() {
            return false;
        }
        if let Some(role) = role {
            if role != self.stream_role {
                self.stream_role = role.to_string();
            }
        }
        self.text.push_str(&normalized);
        self.append_to_last("text", &normalized, role);
        true
    }

    fn append_to_last(&mut self, kind: &str, text: &str, role: Option<&str>) {
        if let Some(last) = self.segments.last_mut() {
            if last.get("kind").and_then(Value::as_str) == Some(kind) && role_of(last) == role {
                // In place: rebuilding the string per delta made a long reply quadratic, and
                // this runs inside Tauri's emit.
                if let Some(Value::String(prev)) = last.get_mut("text") {
                    prev.push_str(text);
                    return;
                }
            }
        }
        let mut seg = segment(kind, role);
        seg.insert("text".into(), json!(text));
        self.segments.push(Value::Object(seg));
    }

    fn upsert_tool(&mut self, id: &str, tc: Value) {
        if !self.tools.contains_key(id) {
            self.tool_order.push(id.to_string());
        }
        self.tools.insert(id.to_string(), tc);
    }

    /// One `acp/stream` event (`{ kind, ... }`). Returns whether the transcript changed.
    pub(crate) fn apply_event(&mut self, event: &Value, role: Option<&str>) -> bool {
        let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
        let str_field = |k: &str| event.get(k).and_then(Value::as_str);
        match kind {
            "thoughtDelta" => {
                let Some(text) = str_field("text").filter(|t| !t.is_empty()) else {
                    return false;
                };
                let normalized = normalize_newlines(text);
                if normalized.trim().is_empty() {
                    return false;
                }
                self.thought.push_str(&normalized);
                let len = self.thought.chars().count();
                if len > MAX_THOUGHT_CHARS {
                    self.thought = self.thought.chars().skip(len - MAX_THOUGHT_CHARS).collect();
                }
                self.append_to_last("thought", &normalized, role);
                true
            }
            "toolCall" => {
                let Some(id) = str_field("toolCallId") else {
                    return false;
                };
                let mut tc = Map::new();
                tc.insert("toolCallId".into(), json!(id));
                tc.insert("title".into(), json!(str_field("title").unwrap_or("")));
                if let Some(name) = str_field("toolName") {
                    tc.insert("toolName".into(), json!(name));
                }
                tc.insert(
                    "kind".into(),
                    json!(str_field("toolKind").unwrap_or("unknown")),
                );
                tc.insert(
                    "status".into(),
                    json!(str_field("status")
                        .map(normalize_tool_status)
                        .unwrap_or_else(|| "pending".into())),
                );
                if let Some(content) = event.get("content").filter(|c| c.is_array()) {
                    tc.insert("content".into(), content.clone());
                }
                if let Some(locations) = normalize_locations(event.get("locations")) {
                    tc.insert("locations".into(), locations);
                }
                for key in ["rawInput", "rawOutput", "terminalMeta", "parentId", "diff"] {
                    if let Some(v) = non_null(event.get(key)) {
                        tc.insert(key.into(), v);
                    }
                }
                if let Some(role) = role {
                    tc.insert("roleName".into(), json!(role));
                }
                let tc = Value::Object(tc);
                self.upsert_tool(id, tc.clone());
                match self
                    .segments
                    .iter()
                    .position(|s| is_tool_segment_for(s, id))
                {
                    Some(idx) => self.segments[idx] = tool_segment(tc, role),
                    None => self.segments.push(tool_segment(tc, role)),
                }
                true
            }
            "toolCallUpdate" => {
                let Some(id) = str_field("toolCallId") else {
                    return false;
                };
                let is_new = !self.tools.contains_key(id);
                let mut tc = match self.tools.get(id) {
                    Some(Value::Object(existing)) => existing.clone(),
                    _ => {
                        let mut base = Map::new();
                        base.insert("toolCallId".into(), json!(id));
                        base.insert("title".into(), json!(str_field("title").unwrap_or("")));
                        base.insert(
                            "kind".into(),
                            json!(str_field("toolKind").unwrap_or("unknown")),
                        );
                        base.insert("status".into(), json!("pending"));
                        base
                    }
                };
                if !tc.contains_key("roleName") {
                    if let Some(role) = role {
                        tc.insert("roleName".into(), json!(role));
                    }
                }
                if let Some(name) = str_field("toolName") {
                    tc.insert("toolName".into(), json!(name));
                }
                if let Some(kind) = str_field("toolKind") {
                    tc.insert("kind".into(), json!(kind));
                }
                if let Some(status) = str_field("status") {
                    tc.insert("status".into(), json!(normalize_tool_status(status)));
                }
                if let Some(title) = str_field("title") {
                    tc.insert("title".into(), json!(title));
                }
                if let Some(content) = event.get("content").filter(|c| c.is_array()) {
                    tc.insert("content".into(), content.clone());
                }
                if let Some(locations) = normalize_locations(event.get("locations")) {
                    tc.insert("locations".into(), locations);
                }
                for key in ["rawInput", "rawOutput", "terminalMeta", "parentId", "diff"] {
                    if let Some(v) = non_null(event.get(key)) {
                        tc.insert(key.into(), v);
                    }
                }
                let tc = Value::Object(tc);
                self.upsert_tool(id, tc.clone());
                if is_new {
                    self.segments.push(tool_segment(tc, role));
                } else if let Some(idx) = self
                    .segments
                    .iter()
                    .rposition(|s| is_tool_segment_for(s, id))
                {
                    let seg_role = role_of(&self.segments[idx]).map(str::to_string);
                    self.segments[idx] = tool_segment(tc, seg_role.as_deref().or(role));
                }
                true
            }
            "toolOutputDelta" => {
                let (Some(id), Some(delta)) = (str_field("toolCallId"), str_field("delta")) else {
                    return false;
                };
                if delta.is_empty() {
                    return false;
                }
                let Some(Value::Object(existing)) = self.tools.get(id) else {
                    return false;
                };
                let mut tc = existing.clone();
                let log = tc.get("outputLog").and_then(Value::as_str).unwrap_or("");
                tc.insert("outputLog".into(), json!(format!("{log}{delta}")));
                let tc = Value::Object(tc);
                self.tools.insert(id.to_string(), tc.clone());
                if let Some(idx) = self
                    .segments
                    .iter()
                    .position(|s| is_tool_segment_for(s, id))
                {
                    let seg_role = role_of(&self.segments[idx]).map(str::to_string);
                    self.segments[idx] = tool_segment(tc, seg_role.as_deref().or(role));
                }
                true
            }
            k if UI_ONLY_KINDS.contains(&k) => false,
            "unknown" => {
                let mut seg = segment("other", role);
                seg.insert(
                    "type".into(),
                    json!(str_field("typeName").unwrap_or("unknown")),
                );
                seg.insert(
                    "payload".into(),
                    non_null(event.get("raw")).unwrap_or_else(|| event.clone()),
                );
                self.segments.push(Value::Object(seg));
                true
            }
            other => {
                let mut seg = segment("other", role);
                seg.insert(
                    "type".into(),
                    json!(if other.is_empty() { "unknown" } else { other }),
                );
                seg.insert("payload".into(), event.clone());
                self.segments.push(Value::Object(seg));
                true
            }
        }
    }

    /// Close tool calls the provider never reported a final status for, so a persisted message
    /// never reloads with a spinner-state tool. Returns whether anything changed.
    pub(crate) fn settle_open_tools(&mut self, status: &str) -> bool {
        fn settle(tc: &mut Value, status: &str) -> bool {
            let open = match tc.get("status").and_then(Value::as_str) {
                Some(s) => !matches!(
                    normalize_tool_status(s).as_str(),
                    "completed" | "failure" | "cancelled" | "interrupted"
                ),
                None => true,
            };
            if open {
                tc["status"] = json!(status);
            }
            open
        }
        let mut changed = false;
        for tc in self.tools.values_mut() {
            changed |= settle(tc, status);
        }
        for seg in &mut self.segments {
            if let Some(tc) = seg.get_mut("tc") {
                changed |= settle(tc, status);
            }
        }
        changed
    }

    fn ordered_tools(&self) -> Vec<Value> {
        self.tool_order
            .iter()
            .filter_map(|id| self.tools.get(id).cloned())
            .collect()
    }

    /// The single-role message for this turn. `final_reply` replaces the streamed text (it is
    /// the authoritative reply once the agent finishes); `None` keeps whatever streamed, which
    /// is what checkpoints and failed/cancelled turns want.
    pub(crate) fn message(&self, id: &str, at: i64, final_reply: Option<&str>) -> Value {
        let tools = self.ordered_tools();
        let mut segments = self.segments.clone();
        if segments.is_empty() && !tools.is_empty() {
            segments = tools
                .iter()
                .map(|tc| {
                    let role = tc
                        .get("roleName")
                        .and_then(Value::as_str)
                        .unwrap_or(&self.fallback_role);
                    tool_segment(tc.clone(), Some(role))
                })
                .collect();
        }
        if let Some(reply) = final_reply {
            if !segments.is_empty() {
                let normalized = normalize_newlines(reply);
                let last_is_text = segments
                    .last()
                    .and_then(|s| s.get("kind"))
                    .and_then(Value::as_str)
                    == Some("text");
                if last_is_text {
                    if let Some(last) = segments.last_mut() {
                        last["text"] = json!(normalized);
                    }
                } else {
                    segments.push(json!({ "kind": "text", "text": normalized }));
                }
            }
        }
        let mut msg = json!({
            "id": id,
            "roleName": self.stream_role,
            "messageType": "assistant",
            "purpose": "response",
            "text": normalize_newlines(final_reply.unwrap_or(&self.text)),
            "at": at,
        });
        if !tools.is_empty() {
            msg["toolCalls"] = Value::Array(tools);
        }
        if !segments.is_empty() {
            msg["segments"] = Value::Array(segments);
        }
        if !self.thought.is_empty() {
            msg["thoughtText"] = json!(self.thought);
        }
        msg
    }

    /// Final messages for the turn, mirroring the desktop's end-of-turn rules:
    /// a multi-agent turn is split per role, a failed turn keeps only what streamed, and a
    /// successful single-role turn is always recorded (even with an empty reply).
    pub(crate) fn finish(&self, id: &str, at: i64, outcome: &TurnOutcome<'_>) -> Vec<Value> {
        match outcome {
            TurnOutcome::Reply { role_replies, .. } if role_replies.len() > 1 => {
                let first_role = role_replies[0].role_name.as_str();
                // Frames some adapters send without a role belong to someone; give them to the
                // first persona rather than dropping them from every split message.
                let owner = |seg_role: Option<&str>| {
                    seg_role
                        .filter(|r| role_replies.iter().any(|reply| reply.role_name == *r))
                        .unwrap_or(first_role)
                        .to_string()
                };
                let mut seen: HashMap<&str, usize> = HashMap::new();
                role_replies
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let role = r.role_name.as_str();
                        let segments: Vec<Value> =
                            self.segments
                                .iter()
                                .filter(|s| {
                                    owner(role_of(s).or_else(|| {
                                        s.pointer("/tc/roleName").and_then(Value::as_str)
                                    })) == role
                                })
                                .cloned()
                                .collect();
                        let tools: Vec<Value> = self
                            .ordered_tools()
                            .into_iter()
                            .filter(|tc| owner(tc.get("roleName").and_then(Value::as_str)) == role)
                            .collect();
                        // The first role reuses the turn id so the streaming checkpoint row is
                        // overwritten instead of left behind as a duplicate; a persona mentioned
                        // twice still gets distinct ids.
                        let repeat = seen.entry(role).or_insert(0);
                        *repeat += 1;
                        let msg_id = match (i, *repeat) {
                            (0, _) => id.to_string(),
                            (_, 1) => format!("{id}:{role}"),
                            (_, n) => format!("{id}:{role}:{n}"),
                        };
                        let mut msg = json!({
                            "id": msg_id,
                            "roleName": role,
                            "messageType": "assistant",
                            "purpose": "response",
                            "text": normalize_newlines(&r.reply),
                            "at": at,
                        });
                        if !segments.is_empty() {
                            msg["segments"] = Value::Array(segments);
                        }
                        if !tools.is_empty() {
                            msg["toolCalls"] = Value::Array(tools);
                        }
                        if i == 0 && !self.thought.is_empty() {
                            msg["thoughtText"] = json!(self.thought);
                        }
                        msg
                    })
                    .collect()
            }
            TurnOutcome::Reply {
                ok: true, reply, ..
            } => vec![self.message(id, at, Some(reply))],
            TurnOutcome::Reply { ok: false, .. } | TurnOutcome::Failed => {
                if self.has_content() {
                    vec![self.message(id, at, None)]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// Splits a message into the `(content_type, payload)` columns the same way the desktop's
/// `persistMessage` did: plain text unless it carries structure worth keeping.
pub(crate) fn storage_columns(msg: &Value) -> (Option<&'static str>, Option<String>) {
    let non_empty_array = |k: &str| {
        msg.get(k)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    let has_structured = non_empty_array("toolCalls")
        || msg
            .get("segments")
            .and_then(Value::as_array)
            .is_some_and(|segs| {
                segs.iter()
                    .any(|s| s.get("kind").and_then(Value::as_str) != Some("text"))
            })
        || non_empty_array("images")
        || msg
            .get("thoughtText")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.is_empty())
        || msg.get("messageType").is_some()
        || msg.get("purpose").is_some()
        || msg.get("payload").is_some_and(Value::is_object);
    if !has_structured {
        return (None, None);
    }
    let mut payload = Map::new();
    payload.insert("text".into(), msg.get("text").cloned().unwrap_or(json!("")));
    for key in ["images", "thoughtText", "messageType", "purpose", "payload"] {
        if let Some(v) = msg.get(key) {
            payload.insert(key.into(), v.clone());
        }
    }
    if let Some(tools) = msg.get("toolCalls").and_then(Value::as_array) {
        payload.insert(
            "toolCalls".into(),
            Value::Array(tools.iter().map(sanitize_tool_call).collect()),
        );
    }
    if let Some(segments) = msg.get("segments").and_then(Value::as_array) {
        let sanitized = segments
            .iter()
            .map(|seg| match seg.get("tc") {
                Some(tc) if seg.get("kind").and_then(Value::as_str) == Some("tool") => {
                    let mut seg = seg.clone();
                    seg["tc"] = sanitize_tool_call(tc);
                    seg
                }
                _ => seg.clone(),
            })
            .collect();
        payload.insert("segments".into(), Value::Array(sanitized));
    }
    (Some("json"), Some(Value::Object(payload).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RoleReply;

    fn kinds(msg: &Value) -> Vec<String> {
        msg["segments"]
            .as_array()
            .map(|segs| {
                segs.iter()
                    .map(|s| s["kind"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn text_deltas_merge_per_role_and_normalize_newlines() {
        let mut t = Transcript::new("Developer");
        t.apply_text("hel", Some("Developer"));
        t.apply_text("lo\r\n", Some("Developer"));
        t.apply_text("hi", Some("Reviewer"));
        let msg = t.message("m1", 1, None);
        assert_eq!(msg["text"], "hello\nhi");
        assert_eq!(msg["segments"][0]["text"], "hello\n");
        assert_eq!(msg["segments"][1]["roleName"], "Reviewer");
        assert_eq!(
            msg["roleName"], "Reviewer",
            "follows the streaming role like the desktop"
        );
    }

    #[test]
    fn tool_call_then_update_upserts_in_place() {
        let mut t = Transcript::new("Developer");
        t.apply_text("before ", Some("Developer"));
        t.apply_event(
            &json!({"kind": "toolCall", "toolCallId": "t1", "title": "Read", "toolKind": "read", "status": "in_progress"}),
            Some("Developer"),
        );
        t.apply_text("after", Some("Developer"));
        t.apply_event(
            &json!({"kind": "toolCallUpdate", "toolCallId": "t1", "status": "completed", "rawOutput": {"ok": true}}),
            Some("Developer"),
        );
        t.apply_event(
            &json!({"kind": "toolOutputDelta", "toolCallId": "t1", "delta": "line1\n"}),
            Some("Developer"),
        );
        let msg = t.message("m1", 1, Some("final"));
        assert_eq!(kinds(&msg), ["text", "tool", "text"]);
        let tc = &msg["toolCalls"][0];
        assert_eq!(tc["status"], "completed");
        assert_eq!(tc["title"], "Read");
        assert_eq!(tc["rawOutput"]["ok"], true);
        assert_eq!(tc["outputLog"], "line1\n");
        assert_eq!(msg["segments"][1]["tc"]["status"], "completed");
        assert_eq!(
            msg["segments"][2]["text"], "final",
            "final reply replaces trailing text"
        );
        assert_eq!(msg["text"], "final");
    }

    #[test]
    fn final_reply_is_appended_when_turn_ends_on_a_tool() {
        let mut t = Transcript::new("Developer");
        t.apply_event(
            &json!({"kind": "toolCall", "toolCallId": "t1", "title": "Run"}),
            Some("Developer"),
        );
        let msg = t.message("m1", 1, Some("done"));
        assert_eq!(kinds(&msg), ["tool", "text"]);
    }

    #[test]
    fn ui_only_events_do_not_touch_the_transcript() {
        let mut t = Transcript::new("Developer");
        assert!(!t.apply_event(&json!({"kind": "usage", "inputTokens": 3}), None));
        assert!(!t.apply_event(
            &json!({"kind": "permissionRequest", "requestId": "r"}),
            None
        ));
        assert!(!t.has_content());
        assert!(t.apply_event(&json!({"kind": "sessionInfo", "title": "x"}), None));
        assert_eq!(
            t.message("m", 1, None)["segments"][0]["type"],
            "sessionInfo"
        );
    }

    #[test]
    fn thought_is_capped_but_segments_keep_everything() {
        let mut t = Transcript::new("Developer");
        let chunk = "a".repeat(3000);
        t.apply_event(
            &json!({"kind": "thoughtDelta", "text": chunk}),
            Some("Developer"),
        );
        t.apply_event(
            &json!({"kind": "thoughtDelta", "text": chunk}),
            Some("Developer"),
        );
        let msg = t.message("m", 1, None);
        assert_eq!(
            msg["thoughtText"].as_str().unwrap().len(),
            MAX_THOUGHT_CHARS
        );
        assert_eq!(msg["segments"][0]["text"].as_str().unwrap().len(), 6000);
    }

    #[test]
    fn settle_closes_open_tools_in_tools_and_segments() {
        let mut t = Transcript::new("Developer");
        for (id, st) in [("a", "in_progress"), ("b", "pending"), ("c", "completed")] {
            t.apply_event(
                &json!({"kind": "toolCall", "toolCallId": id, "title": id, "status": st}),
                Some("Developer"),
            );
        }
        assert!(t.settle_open_tools("cancelled"));
        let msg = t.message("m", 1, None);
        let st: Vec<&str> = msg["toolCalls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tc| tc["status"].as_str().unwrap())
            .collect();
        assert_eq!(st, ["cancelled", "cancelled", "completed"]);
        assert_eq!(msg["segments"][0]["tc"]["status"], "cancelled");
        assert_eq!(msg["segments"][2]["tc"]["status"], "completed");
        assert!(!t.settle_open_tools("failure"));
    }

    #[test]
    fn event_payload_is_persisted_as_structured_column() {
        let row = json!({"id": "t:error", "roleName": "event", "text": "x", "payload": {"kind": "turnError", "code": "AUTH_REQUIRED"}});
        let (ct, p) = storage_columns(&row);
        assert_eq!(ct, Some("json"));
        assert!(p.unwrap().contains("AUTH_REQUIRED"));
    }

    #[test]
    fn failed_turn_keeps_partial_content_or_records_nothing() {
        let empty = Transcript::new("Developer");
        assert!(empty.finish("m", 1, &TurnOutcome::Failed).is_empty());
        let mut partial = Transcript::new("Developer");
        partial.apply_text("half", Some("Developer"));
        let msgs = partial.finish(
            "m",
            1,
            &TurnOutcome::Reply {
                ok: false,
                reply: "Error: boom",
                role_replies: &[],
            },
        );
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0]["text"], "half",
            "an error reply never overwrites streamed content"
        );
    }

    #[test]
    fn successful_turn_is_recorded_even_when_empty() {
        let t = Transcript::new("Developer");
        let msgs = t.finish(
            "m",
            1,
            &TurnOutcome::Reply {
                ok: true,
                reply: "",
                role_replies: &[],
            },
        );
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["id"], "m");
    }

    #[test]
    fn multi_agent_turn_splits_per_role_and_reuses_turn_id_first() {
        let mut t = Transcript::new("Developer & Reviewer");
        t.apply_text("dev says", Some("Developer"));
        t.apply_event(
            &json!({"kind": "toolCall", "toolCallId": "t1", "title": "Edit"}),
            Some("Developer"),
        );
        t.apply_text("rev says", Some("Reviewer"));
        let replies = vec![
            RoleReply {
                role_name: "Developer".into(),
                reply: "dev".into(),
                ok: true,
                error_code: None,
            },
            RoleReply {
                role_name: "Reviewer".into(),
                reply: "rev".into(),
                ok: true,
                error_code: None,
            },
        ];
        let msgs = t.finish(
            "m",
            1,
            &TurnOutcome::Reply {
                ok: true,
                reply: "",
                role_replies: &replies,
            },
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["id"], "m");
        assert_eq!(msgs[1]["id"], "m:Reviewer");
        assert_eq!(kinds(&msgs[0]), ["text", "tool"]);
        assert_eq!(msgs[0]["toolCalls"].as_array().unwrap().len(), 1);
        assert_eq!(kinds(&msgs[1]), ["text"]);
        assert!(msgs[1].get("toolCalls").is_none());
    }

    #[test]
    fn multi_agent_split_keeps_roleless_frames_and_thought_with_the_first_persona() {
        let mut t = Transcript::new("Developer & Reviewer");
        t.apply_text("no role", None);
        t.apply_event(
            &json!({"kind": "thoughtDelta", "text": "hmm"}),
            Some("Developer"),
        );
        t.apply_text("rev", Some("Reviewer"));
        let replies = vec![
            RoleReply {
                role_name: "Developer".into(),
                reply: "d".into(),
                ok: true,
                error_code: None,
            },
            RoleReply {
                role_name: "Reviewer".into(),
                reply: "r".into(),
                ok: true,
                error_code: None,
            },
            RoleReply {
                role_name: "Reviewer".into(),
                reply: "r2".into(),
                ok: true,
                error_code: None,
            },
        ];
        let msgs = t.finish(
            "m",
            1,
            &TurnOutcome::Reply {
                ok: true,
                reply: "",
                role_replies: &replies,
            },
        );
        assert_eq!(
            kinds(&msgs[0]),
            ["text", "thought"],
            "role-less text stays with the first persona"
        );
        assert_eq!(msgs[0]["thoughtText"], "hmm");
        assert_eq!(msgs[1]["id"], "m:Reviewer");
        assert_eq!(
            msgs[2]["id"], "m:Reviewer:2",
            "repeated persona gets a distinct id"
        );
    }

    #[test]
    fn storage_columns_keep_plain_text_plain_and_sanitize_tools() {
        let plain = json!({"id": "m", "roleName": "user", "text": "hi", "at": 1});
        assert_eq!(storage_columns(&plain), (None, None));

        let big = "x".repeat(OUTPUT_LOG_LIMIT + 10);
        let msg = json!({
            "text": "t",
            "toolCalls": [{"toolCallId": "t1", "contentJson": "{}", "outputLog": big}],
        });
        let (ct, payload) = storage_columns(&msg);
        assert_eq!(ct, Some("json"));
        let payload: Value = serde_json::from_str(&payload.unwrap()).unwrap();
        let tc = &payload["toolCalls"][0];
        assert!(tc.get("contentJson").is_none());
        assert!(tc["outputLog"].as_str().unwrap().contains("[truncated]"));
        assert!(tc["outputLog"].as_str().unwrap().len() <= 2 * OUTPUT_LOG_KEEP + 32);
    }
}
