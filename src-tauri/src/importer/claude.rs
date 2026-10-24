use super::tail::{parse_iso_ms, Acc, Item, Outcome, Reduce, Unit};
use super::DiscoveredSession;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub(crate) fn encode_claude_project_dir(path: &str) -> String {
    let canonical = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());

    let mut encoded: String = canonical
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    if encoded.len() > 200 {
        let mut hash: i32 = 0;
        for b in canonical.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(b as i32);
        }
        let hash_str = format!("{:x}", hash.abs());
        encoded.truncate(200);
        format!("{}-{}", encoded, hash_str)
    } else {
        encoded
    }
}

fn claude_projects_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return None;
    }
    let config_dir =
        std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| format!("{}/.claude", home));
    Some(Path::new(&config_dir).join("projects"))
}

/// Every Claude transcript on the machine, whatever directory it ran in.
pub(crate) fn scan_all_claude_sessions() -> Vec<DiscoveredSession> {
    claude_projects_dir()
        .map(|dir| scan_all_claude_sessions_in(&dir))
        .unwrap_or_default()
}

fn scan_all_claude_sessions_in(projects_dir: &Path) -> Vec<DiscoveredSession> {
    let dirs = std::fs::read_dir(projects_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| (entry.path(), true))
        .collect();
    scan_claude_dirs(dirs, None)
}

pub(crate) fn scan_claude_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    let Some(projects_dir) = claude_projects_dir() else {
        return Vec::new();
    };
    let encoded = encode_claude_project_dir(project_path);
    let project_root = canonical_project_path(project_path);

    // Claude keeps one directory per working directory, so sessions started in a
    // sub-directory of the project live under `<encoded>-<sub>`. Encoding is lossy (`/` and
    // `-` both become `-`), so such a directory only counts once the transcript's own `cwd`
    // confirms it is inside the project — `proj-other` must not match `proj`.
    let mut dirs: Vec<(PathBuf, bool)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == encoded {
                dirs.push((entry.path(), false));
            } else if name.starts_with(&format!("{encoded}-")) {
                dirs.push((entry.path(), true));
            }
        }
    }

    scan_claude_dirs(dirs, Some(&project_root))
}

/// `dirs` pairs a transcript directory with whether the cwd must come from the transcript
/// itself. With a `project_root`, only transcripts inside it are kept.
fn scan_claude_dirs(
    dirs: Vec<(PathBuf, bool)>,
    project_root: Option<&str>,
) -> Vec<DiscoveredSession> {
    let mut sessions = Vec::new();
    for (dir, verify_cwd) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(session_id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let cwd = if verify_cwd {
                match (transcript_cwd(&path), project_root) {
                    (Some(cwd), Some(root)) if Path::new(&cwd).starts_with(root) => cwd,
                    (Some(cwd), None) => cwd,
                    _ => continue,
                }
            } else {
                project_root.unwrap_or_default().to_string()
            };
            let Some((title, turn_count)) = inspect_claude_session(&path, &session_id) else {
                continue;
            };
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or_else(crate::now_ms);
            sessions.push(DiscoveredSession {
                runtime_kind: "claude-code",
                agent: "Claude Code",
                session_id,
                file_path: path,
                // Resuming needs the directory the session ran in, not the project root.
                cwd,
                title,
                created_at: mtime,
                last_active_at: mtime,
                turn_count,
            });
        }
    }
    sessions.sort_by_key(|b| std::cmp::Reverse(b.last_active_at));
    sessions
}

/// The working directory a Claude transcript recorded (every entry carries `cwd`).
fn transcript_cwd(path: &Path) -> Option<String> {
    let reader = BufReader::new(File::open(path).ok()?);
    reader
        .lines()
        .take(50)
        .map_while(Result::ok)
        .filter(|line| line.contains("\"cwd\""))
        .find_map(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()?
                .get("cwd")?
                .as_str()
                .map(str::to_string)
        })
}

fn canonical_project_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
pub(crate) fn count_claude_session_turns(path: &Path) -> usize {
    inspect_claude_session(path, "")
        .map(|(_, turns)| turns)
        .unwrap_or(0)
}

fn clean_claude_title(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text).trim();
    let cleaned: String = first_line
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    cleaned.trim().to_string()
}

fn tag_body<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{tag}>"))? + start;
    Some(text[start..end].trim())
}

/// The CLI wakes the model with a `<task-notification>` block stored as a user-role entry
/// (`origin.kind == "task-notification"`) when a background task or subagent finishes. It is
/// not human input; it is surfaced as a compact event row instead of a user bubble.
fn task_notification_event(entry: &serde_json::Value, text: &str) -> Option<String> {
    let flagged =
        entry.pointer("/origin/kind").and_then(|k| k.as_str()) == Some("task-notification");
    let trimmed = text.trim();
    if !flagged && !trimmed.starts_with("<task-notification>") {
        return None;
    }
    let status = tag_body(trimmed, "status")
        .filter(|s| !s.is_empty())
        .unwrap_or("finished");
    let summary = tag_body(trimmed, "summary").filter(|s| !s.is_empty());
    let unescape = |s: &str| {
        s.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
    };
    Some(match summary {
        Some(summary) => format!("Background task {status}: {}", unescape(summary)),
        None => format!("Background task {status}"),
    })
}

fn clean_claude_user_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with("<task-notification>") {
        return None;
    }
    if trimmed.starts_with("<local-command-")
        || trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<system-reminder>")
        || trimmed.starts_with("Caveat:")
    {
        return None;
    }
    let after_context = if let Some(idx) = trimmed.find("User:\n") {
        trimmed[idx + 6..].trim()
    } else if trimmed.starts_with("System:\n[Jockey context]") {
        return None;
    } else {
        trimmed
    };
    if after_context.is_empty()
        || after_context.starts_with("<local-command-")
        || after_context.starts_with("<command-name>")
        || after_context.starts_with("<system-reminder>")
        || after_context.starts_with("Caveat:")
    {
        return None;
    }
    Some(after_context.to_string())
}

fn inspect_claude_session(path: &Path, session_id: &str) -> Option<(String, usize)> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut ai_title: Option<String> = None;
    let mut first_user_prompt: Option<String> = None;
    let mut user_turns: usize = 0;
    let mut assistant_turns: usize = 0;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.trim().is_empty() {
            continue;
        }

        let has_ai_title = line.contains("\"aiTitle\"")
            || line.contains("\"agentName\"")
            || line.contains("\"customTitle\"");
        let has_user = line.contains("\"type\":\"user\"");
        let has_assistant = line.contains("\"type\":\"assistant\"");

        if !has_ai_title && !has_user && !has_assistant {
            continue;
        }

        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        if let Some(t) = val.get("aiTitle").and_then(|t| t.as_str()) {
            let trimmed = t.trim();
            if !trimmed.is_empty() && trimmed != "Untitled session" {
                ai_title = Some(clean_claude_title(trimmed));
            }
        }
        if let Some(t) = val.get("agentName").and_then(|t| t.as_str()) {
            let trimmed = t.trim();
            if !trimmed.is_empty() && trimmed != "Untitled session" {
                ai_title = Some(clean_claude_title(trimmed));
            }
        }
        if let Some(t) = val.get("customTitle").and_then(|t| t.as_str()) {
            let trimmed = t.trim();
            if !trimmed.is_empty() && trimmed != "Untitled session" {
                ai_title = Some(clean_claude_title(trimmed));
            }
        }

        if val.get("type").and_then(|t| t.as_str()) == Some("user") {
            if val.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
                || val.get("isCompactSummary").and_then(|m| m.as_bool()) == Some(true)
            {
                continue;
            }

            let mut raw_text = String::new();
            if let Some(content) = val.pointer("/message/content").and_then(|c| c.as_str()) {
                raw_text.push_str(content);
            } else if let Some(arr) = val.pointer("/message/content").and_then(|c| c.as_array()) {
                for b in arr {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(t) = b.get("text").and_then(|s| s.as_str()) {
                            raw_text.push_str(t);
                        }
                    }
                }
            }

            if val.pointer("/origin/kind").and_then(|k| k.as_str()) == Some("task-notification") {
                continue;
            }
            if let Some(cleaned) = clean_claude_user_text(&raw_text) {
                user_turns += 1;
                if first_user_prompt.is_none() {
                    first_user_prompt = Some(clean_claude_title(&cleaned));
                }
            }
        } else if val.get("type").and_then(|t| t.as_str()) == Some("assistant") {
            if val.get("message").is_some() {
                assistant_turns += 1;
            }
        }
    }

    if user_turns == 0 && assistant_turns == 0 && ai_title.is_none() {
        return None;
    }

    let title = ai_title.or(first_user_prompt).unwrap_or_else(|| {
        if !session_id.is_empty() {
            format!("Claude_{}", &session_id[..8.min(session_id.len())])
        } else {
            "Claude session".to_string()
        }
    });

    Some((title, user_turns + assistant_turns))
}

/// Entries that never carry conversation content, rejected before JSON parsing: the API
/// request mirrors are by far the largest lines in a transcript. Their `"type":"..."` appears
/// unescaped only at the top level, so message text can never match.
pub(crate) fn skip_line(line: &[u8]) -> bool {
    const SKIPPED: [&[u8]; 3] = [
        b"\"type\":\"api-request",
        b"\"type\":\"attachment\"",
        b"\"type\":\"file-history",
    ];
    SKIPPED
        .iter()
        .any(|needle| line.windows(needle.len()).any(|w| w == *needle))
}

/// Groups Claude Code entries into Jockey-shaped items. Claude writes one entry per content
/// block, repeating the message's `stop_reason` on each of them, so a turn is closed by the
/// entry that carries the reply (not a thinking block that precedes it), by the CLI's
/// `turn_duration` marker, by an interruption, or by the next prompt.
pub(crate) struct ClaudeReducer {
    acc: Acc,
}

impl ClaudeReducer {
    pub(crate) fn new(role: String) -> Self {
        Self {
            acc: Acc::new(role),
        }
    }

    fn text_of(content: Option<&serde_json::Value>) -> String {
        match content {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(blocks)) => blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<String>(),
            _ => String::new(),
        }
    }

    fn user(&mut self, unit: &Unit, at: i64) {
        let entry = &unit.value;
        if entry.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
            || entry.get("isCompactSummary").and_then(|m| m.as_bool()) == Some(true)
        {
            return;
        }
        let content = entry.pointer("/message/content");
        let mut had_result = false;
        if let Some(blocks) = content.and_then(|c| c.as_array()) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                    continue;
                }
                had_result = true;
                if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                    let is_error = block.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                    self.acc.tool_result(
                        id,
                        block.get("content").unwrap_or(&serde_json::Value::Null),
                        is_error,
                    );
                }
            }
        }
        if had_result {
            self.acc.running = Some(true);
            return;
        }
        let text = Self::text_of(content);
        let id = entry_id(entry, unit.start);
        if let Some(summary) = task_notification_event(entry, &text) {
            self.acc
                .push(Item::plain(format!("e:{id}"), "event", summary, at));
            self.acc.running = Some(true);
            return;
        }
        if text.trim().starts_with("[Request interrupted by user") {
            self.acc.close_turn();
            self.acc.running = Some(false);
            return;
        }
        if let Some(cleaned) = clean_claude_user_text(&text) {
            self.acc.close_turn();
            self.acc
                .push(Item::plain(format!("u:{id}"), "user", cleaned, at));
            self.acc.running = Some(true);
        }
    }

    fn assistant(&mut self, unit: &Unit, at: i64) {
        let entry = &unit.value;
        let Some(message) = entry.get("message") else {
            return;
        };
        let id = entry_id(entry, unit.start);
        self.acc.ensure_turn(|| format!("t:{id}"), unit.start, at);
        let mut only_thinking = true;
        match message.get("content") {
            Some(serde_json::Value::String(text)) => {
                only_thinking = false;
                self.acc.text(text);
            }
            Some(serde_json::Value::Array(blocks)) => {
                for block in blocks {
                    let kind = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if kind != "thinking" && kind != "redacted_thinking" {
                        only_thinking = false;
                    }
                    match kind {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                self.acc.text(text);
                            }
                        }
                        "thinking" => {
                            if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                                self.acc.thought(text);
                            }
                        }
                        "tool_use" => {
                            if let Some(tool_id) = block.get("id").and_then(|i| i.as_str()) {
                                let name =
                                    block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                                self.acc.tool_call(
                                    tool_id,
                                    name,
                                    block.get("input").unwrap_or(&serde_json::Value::Null),
                                    None,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let finished = matches!(
            message.get("stop_reason").and_then(|s| s.as_str()),
            Some(reason) if reason != "tool_use" && reason != "pause_turn"
        );
        if finished && !only_thinking {
            self.acc.close_turn();
            self.acc.running = Some(false);
        } else {
            self.acc.running = Some(true);
        }
    }
}

fn entry_id(entry: &serde_json::Value, offset: u64) -> String {
    entry
        .get("uuid")
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("o{offset}"))
}

impl Reduce for ClaudeReducer {
    fn feed(&mut self, unit: &Unit) {
        let entry = &unit.value;
        let at = self.acc.stamp(
            entry
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(parse_iso_ms),
        );
        if entry.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
            return;
        }
        match entry.get("type").and_then(|t| t.as_str()) {
            Some("user") => self.user(unit, at),
            Some("assistant") => self.assistant(unit, at),
            Some("system")
                if entry.get("subtype").and_then(|s| s.as_str()) == Some("turn_duration") =>
            {
                self.acc.close_turn();
                self.acc.running = Some(false);
            }
            _ => {}
        }
    }

    fn finish(self: Box<Self>, consumed: u64, stale: bool) -> Outcome {
        self.acc.finish(consumed, stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[serde_json::Value]) -> Outcome {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sess.jsonl");
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, text).expect("write fixture");
        crate::importer::tail::read("claude-code", "Developer", &path, 0, false)
            .unwrap()
            .unwrap()
    }

    fn rows(out: &Outcome) -> Vec<(&str, &str)> {
        out.items
            .iter()
            .map(|m| (m.role.as_str(), m.text.as_str()))
            .collect()
    }

    fn user_entry(uuid: &str, text: &str, ts: &str) -> serde_json::Value {
        serde_json::json!({"type":"user","uuid":uuid,"timestamp":ts,"origin":{"kind":"human"},
            "message":{"role":"user","content":text}})
    }

    fn assistant_entry(
        uuid: &str,
        stop: &str,
        blocks: serde_json::Value,
        ts: &str,
    ) -> serde_json::Value {
        serde_json::json!({"type":"assistant","uuid":uuid,"timestamp":ts,
            "message":{"role":"assistant","id":"msg_1","stop_reason":stop,"content":blocks}})
    }

    fn tool_result(uuid: &str, tool_id: &str, text: &str, ts: &str) -> serde_json::Value {
        serde_json::json!({"type":"user","uuid":uuid,"timestamp":ts,"message":{"role":"user",
            "content":[{"type":"tool_result","tool_use_id":tool_id,"content":text}]}})
    }

    #[test]
    fn task_notifications_become_event_rows_not_user_messages() {
        let note = "<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>\n<summary>Agent &quot;x&quot; done</summary>\n</task-notification>";
        let flagged = serde_json::json!({"type":"user","uuid":"n1","origin":{"kind":"task-notification"},
            "message":{"role":"user","content":note}});
        let bare =
            serde_json::json!({"type":"user","uuid":"n2","message":{"role":"user","content":note}});
        let human = serde_json::json!({"type":"user","uuid":"h1","origin":{"kind":"human"},
            "message":{"role":"user","content":"go on"}});
        let out = run(&[flagged, bare, human]);
        assert_eq!(
            rows(&out),
            vec![
                ("event", "Background task completed: Agent \"x\" done"),
                ("event", "Background task completed: Agent \"x\" done"),
                ("user", "go on"),
            ]
        );
        assert!(clean_claude_user_text(note).is_none());
    }

    #[test]
    fn a_turn_becomes_one_message_with_tool_segments_and_ends_on_the_reply() {
        let out = run(&[
            user_entry("u1", "list files", "2026-10-20T00:00:00.000Z"),
            serde_json::json!({"type":"attachment","uuid":"x"}),
            assistant_entry(
                "a1",
                "tool_use",
                serde_json::json!([{"type":"thinking","thinking":"hmm"}]),
                "2026-10-20T00:00:01.000Z",
            ),
            assistant_entry(
                "a2",
                "tool_use",
                serde_json::json!([{"type":"text","text":"Looking."},
                {"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls","description":"List files"}}]),
                "2026-10-20T00:00:02.000Z",
            ),
            tool_result("r1", "tu1", "a.txt", "2026-10-20T00:00:03.000Z"),
            assistant_entry(
                "a3",
                "end_turn",
                serde_json::json!([{"type":"text","text":"Done: a.txt"}]),
                "2026-10-20T00:00:04.000Z",
            ),
        ]);
        assert_eq!(out.items.len(), 2);
        assert!(out.open.is_none());
        assert_eq!(out.running, Some(false));
        let turn = &out.items[1];
        assert_eq!(turn.id, "t:a1");
        assert_eq!(turn.text, "Looking.\n\nDone: a.txt");
        let msg = turn.msg.as_ref().unwrap();
        let kinds: Vec<_> = msg["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["thought", "text", "tool", "text"]);
        let tc = &msg["toolCalls"][0];
        assert_eq!(tc["toolName"], "Bash");
        assert_eq!(tc["kind"], "execute");
        assert_eq!(tc["status"], "completed");
        assert_eq!(tc["rawOutput"], "a.txt");
        assert_eq!(tc["title"], "List files");
        assert_eq!(msg["thoughtText"], "hmm");
    }

    #[test]
    fn an_unfinished_turn_is_held_back_and_resumes_from_its_start() {
        let lines = [
            user_entry("u1", "run it", "2026-10-20T00:00:00.000Z"),
            assistant_entry(
                "a1",
                "tool_use",
                serde_json::json!([
                {"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"sleep 100"}}]),
                "2026-10-20T00:00:01.000Z",
            ),
        ];
        let out = run(&lines);
        assert_eq!(rows(&out), vec![("user", "run it")]);
        assert_eq!(out.running, Some(true));
        let open = out.open.as_ref().expect("live preview");
        assert_eq!(open.id, "t:a1");
        // The next read starts at the open turn, not at the end of the file.
        let first_line = format!("{}\n", lines[0]).len() as u64;
        assert_eq!(out.commit, first_line);
    }

    #[test]
    fn a_thinking_block_marked_end_turn_does_not_close_the_turn_before_the_reply() {
        let out = run(&[
            user_entry("u1", "hi", "2026-10-20T00:00:00.000Z"),
            assistant_entry(
                "a1",
                "end_turn",
                serde_json::json!([{"type":"thinking","thinking":"x"}]),
                "2026-10-20T00:00:01.000Z",
            ),
        ]);
        assert_eq!(out.items.len(), 1);
        assert!(out.open.is_some());
        assert_eq!(out.running, Some(true));
    }

    #[test]
    fn interruptions_and_turn_duration_end_a_turn_and_settle_its_tools() {
        let out = run(&[
            user_entry("u1", "go", "2026-10-20T00:00:00.000Z"),
            assistant_entry(
                "a1",
                "tool_use",
                serde_json::json!([
                {"type":"tool_use","id":"tu1","name":"Read","input":{"file_path":"/a"}}]),
                "2026-10-20T00:00:01.000Z",
            ),
            user_entry(
                "u2",
                "[Request interrupted by user for tool use]",
                "2026-10-20T00:00:02.000Z",
            ),
        ]);
        assert_eq!(out.items.len(), 2, "no row for the interruption marker");
        assert_eq!(
            out.items[1].msg.as_ref().unwrap()["toolCalls"][0]["status"],
            "interrupted"
        );
        assert_eq!(out.running, Some(false));

        let done = run(&[
            user_entry("u1", "go", "2026-10-20T00:00:00.000Z"),
            assistant_entry(
                "a1",
                "end_turn",
                serde_json::json!([{"type":"thinking","thinking":"x"}]),
                "2026-10-20T00:00:01.000Z",
            ),
            serde_json::json!({"type":"system","subtype":"turn_duration","uuid":"s1"}),
        ]);
        assert!(done.open.is_none());
        assert_eq!(done.running, Some(false));
    }

    #[test]
    fn sidechains_and_noise_entries_are_ignored_and_ids_are_stable() {
        let lines = [
            serde_json::json!({"type":"queue-operation","operation":"enqueue"}),
            user_entry("u1", "hi", "2026-10-20T00:00:00.000Z"),
            serde_json::json!({"type":"assistant","uuid":"side","isSidechain":true,"message":{"stop_reason":"end_turn","content":[{"type":"text","text":"child"}]}}),
            assistant_entry(
                "a1",
                "end_turn",
                serde_json::json!([{"type":"text","text":"hello"}]),
                "2026-10-20T00:00:01.000Z",
            ),
        ];
        let first = run(&lines);
        let again = run(&lines);
        let ids = |o: &Outcome| o.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&first), ["u:u1", "t:a1"]);
        assert_eq!(ids(&first), ids(&again));
        assert_eq!(first.items[1].text, "hello");
    }

    #[test]
    fn skip_filter_only_matches_top_level_mirror_entries() {
        assert!(skip_line(br#"{"type":"api-request-blob","hash":"x"}"#));
        assert!(skip_line(br#"{"type":"attachment","uuid":"x"}"#));
        assert!(!skip_line(
            br#"{"type":"user","message":{"content":"say \"type\":\"api-request\""}}"#
        ));
    }

    #[test]
    fn counts_only_message_envelopes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sess.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"summary","summary":"x"}"#,
                "\n",
                r#"{"type":"user","message":{"content":"hi"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"yo"}]}}"#,
                "\n",
                r#"{"type":"user","message":{"content":"again"}}"#,
                "\n",
                "not json\n",
            ),
        )
        .expect("write fixture");
        assert_eq!(count_claude_session_turns(&path), 3);
    }

    #[test]
    fn counts_zero_for_a_missing_file() {
        assert_eq!(
            count_claude_session_turns(Path::new("/nonexistent/nope.jsonl")),
            0
        );
    }

    #[test]
    fn test_encode_and_scan() {
        let encoded = encode_claude_project_dir("/Users/sexy/Documents/GitHub/arise-ops");
        assert_eq!(encoded, "-Users-sexy-Documents-GitHub-arise-ops");
    }

    #[test]
    fn filters_out_slash_command_probe_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("probe.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"isSidechain":false,"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: ...</local-command-caveat>"},"isMeta":true}"#,
                "\n",
                r#"{"parentUuid":"a","isSidechain":false,"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>\n            <command-message>model</command-message>"}}"#,
                "\n",
                r#"{"parentUuid":"b","isSidechain":false,"type":"system","subtype":"local_command","content":"<local-command-stdout>Current model: Opus</local-command-stdout>"}"#,
                "\n",
            ),
        )
        .expect("write probe fixture");

        assert_eq!(inspect_claude_session(&path, "probe"), None);
        assert_eq!(count_claude_session_turns(&path), 0);
    }

    #[test]
    fn discovers_late_ai_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("late.jsonl");
        let mut content = String::new();
        content.push_str(r#"{"type":"user","message":{"content":"claude"}}"#);
        content.push('\n');
        for i in 0..100 {
            content.push_str(&format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"step {}"}}]}}}}"#,
                i
            ));
            content.push('\n');
        }
        content.push_str(r#"{"type":"ai-title","aiTitle":"persona-session-lazy-lifecycle"}"#);
        content.push('\n');
        std::fs::write(&path, &content).expect("write late fixture");

        let summary = inspect_claude_session(&path, "late");
        assert!(summary.is_some());
        let (title, turns) = summary.unwrap();
        assert_eq!(title, "persona-session-lazy-lifecycle");
        assert_eq!(turns, 101);
    }
}

#[cfg(test)]
mod subdir_tests {
    use super::*;

    fn transcript(dir: &Path, id: &str, cwd: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let lines = [
            format!(
                r#"{{"type":"user","cwd":"{cwd}","sessionId":"{id}","message":{{"role":"user","content":"hello"}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","cwd":"{cwd}","sessionId":"{id}","message":{{"role":"assistant","content":[{{"type":"text","text":"hi"}}]}}}}"#
            ),
        ];
        std::fs::write(dir.join(format!("{id}.jsonl")), lines.join("\n")).unwrap();
    }

    #[test]
    fn sub_directory_sessions_are_found_and_siblings_are_not() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("proj");
        std::fs::create_dir_all(project.join("sub")).unwrap();
        std::fs::create_dir_all(root.path().join("proj-other")).unwrap();
        let project = std::fs::canonicalize(&project).unwrap();
        let project_str = project.to_string_lossy().to_string();
        let sibling = std::fs::canonicalize(root.path().join("proj-other")).unwrap();

        let config = tempfile::tempdir().unwrap();
        let projects = config.path().join("projects");
        let enc = encode_claude_project_dir(&project_str);
        transcript(&projects.join(&enc), "root-session", &project_str);
        let sub = project.join("sub").to_string_lossy().to_string();
        transcript(
            &projects.join(encode_claude_project_dir(&sub)),
            "sub-session",
            &sub,
        );
        let other = sibling.to_string_lossy().to_string();
        transcript(
            &projects.join(encode_claude_project_dir(&other)),
            "other-session",
            &other,
        );

        std::env::set_var("CLAUDE_CONFIG_DIR", config.path());
        let found = scan_claude_sessions(&project_str);
        std::env::remove_var("CLAUDE_CONFIG_DIR");

        let mut ids: Vec<_> = found.iter().map(|s| s.session_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["root-session", "sub-session"]);
        let sub_session = found
            .iter()
            .find(|s| s.session_id == "sub-session")
            .unwrap();
        assert_eq!(
            sub_session.cwd, sub,
            "resume needs the directory the session ran in"
        );
    }

    #[test]
    fn scan_all_reads_every_directory_and_its_cwd() {
        let projects = tempfile::tempdir().unwrap();
        transcript(&projects.path().join("-a"), "one", "/work/a");
        transcript(&projects.path().join("-b"), "two", "/work/b");
        let mut found = scan_all_claude_sessions_in(projects.path());
        found.sort_by(|x, y| x.session_id.cmp(&y.session_id));
        let seen: Vec<_> = found
            .iter()
            .map(|s| (s.session_id.as_str(), s.cwd.as_str()))
            .collect();
        assert_eq!(seen, [("one", "/work/a"), ("two", "/work/b")]);
    }
}
