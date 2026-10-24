use super::tail::{parse_iso_ms, Acc, Item, Outcome, Reduce, Unit};
use super::DiscoveredSession;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub(crate) fn scan_all_pi_sessions() -> Vec<DiscoveredSession> {
    sessions_root()
        .map(|root| scan_pi_root(&root, None))
        .unwrap_or_default()
}

pub(crate) fn scan_pi_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    sessions_root()
        .map(|root| scan_pi_root(&root, Some(&canonical_path(project_path))))
        .unwrap_or_default()
}

fn scan_pi_root(root: &Path, project_cwd: Option<&str>) -> Vec<DiscoveredSession> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    files
        .into_iter()
        .filter_map(|file_path| scan_file(&file_path, project_cwd))
        .collect()
}

fn sessions_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".pi")
            .join("agent")
            .join("sessions"),
    )
}

fn clean_pi_user_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let after_context = if let Some(idx) = trimmed.find("User:\n") {
        trimmed[idx + 6..].trim()
    } else if trimmed.starts_with("System:\n[Jockey context]") {
        return None;
    } else {
        trimmed
    };
    if after_context.is_empty() {
        return None;
    }
    Some(after_context.to_string())
}

fn scan_file(path: &Path, project_cwd: Option<&str>) -> Option<DiscoveredSession> {
    let file = File::open(path).ok()?;
    let mut session_id = None;
    let mut cwd = None;
    let mut title = None;
    let mut turn_count = 0;

    for line in BufReader::new(file).lines().map_while(Result::ok).take(200) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session") {
            session_id = value
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            cwd = value.get("cwd").and_then(Value::as_str).map(canonical_path);
            continue;
        }
        if let Some((role, text)) = message_text(&value) {
            if role == "user" {
                if let Some(clean) = clean_pi_user_text(&text) {
                    turn_count += 1;
                    if title.is_none() {
                        title = Some(short_title(&clean));
                    }
                }
            } else if role == "assistant" && !text.trim().is_empty() {
                turn_count += 1;
            }
        }
    }

    let cwd = cwd?;
    // Sessions started in a sub-directory of the project belong to it too.
    if project_cwd.is_some_and(|root| !std::path::Path::new(&cwd).starts_with(root)) {
        return None;
    }
    let turns = count_pi_session_turns(path).max(turn_count);
    if turns == 0 {
        return None;
    }
    let session_id = session_id.or_else(|| path.file_stem()?.to_str().map(ToString::to_string))?;
    let last_active_at = modified_at(path);
    Some(DiscoveredSession {
        runtime_kind: "pi-cli",
        agent: "Pi",
        session_id: session_id.clone(),
        file_path: path.to_path_buf(),
        cwd,
        title: title.unwrap_or_else(|| format!("Pi_{}", short_id(&session_id))),
        created_at: last_active_at,
        last_active_at,
        turn_count: turns,
    })
}

fn count_pi_session_turns(path: &Path) -> usize {
    let Ok(file) = File::open(path) else {
        return 0;
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|value| {
            let (role, text) = message_text(&value)?;
            if role == "user" {
                clean_pi_user_text(&text).map(|_| "user")
            } else if role == "assistant" && !text.trim().is_empty() {
                Some("assistant")
            } else {
                None
            }
        })
        .count()
}

/// Groups Pi session entries into Jockey-shaped items. A Pi assistant message arrives whole
/// with a `stopReason`: `toolUse` means the turn continues with the tool results that follow,
/// anything else (`stop`, `length`, `error`, `aborted`) ends it.
pub(crate) struct PiReducer {
    acc: Acc,
}

impl PiReducer {
    pub(crate) fn new(role: String) -> Self {
        Self {
            acc: Acc::new(role),
        }
    }

    fn assistant(&mut self, unit: &Unit, message: &Value, at: i64) {
        let id = unit
            .value
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| format!("o{}", unit.start), str::to_string);
        self.acc.ensure_turn(|| format!("t:{id}"), unit.start, at);
        match message.get("content") {
            Some(Value::String(text)) => self.acc.text(text),
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                self.acc.text(text);
                            }
                        }
                        Some("thinking") => {
                            if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                                self.acc.thought(text);
                            }
                        }
                        Some("toolCall") => {
                            if let Some(tool_id) = block.get("id").and_then(Value::as_str) {
                                let name =
                                    block.get("name").and_then(Value::as_str).unwrap_or("tool");
                                self.acc.tool_call(
                                    tool_id,
                                    name,
                                    block.get("arguments").unwrap_or(&Value::Null),
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
        let stop = message.get("stopReason").and_then(Value::as_str);
        if stop == Some("error") {
            if let Some(error) = message.get("errorMessage").and_then(Value::as_str) {
                self.acc.text(&format!("Error: {error}"));
            }
        }
        match stop {
            Some("toolUse") | None => self.acc.running = Some(true),
            Some(_) => {
                self.acc.close_turn();
                self.acc.running = Some(false);
            }
        }
    }
}

impl Reduce for PiReducer {
    fn feed(&mut self, unit: &Unit) {
        let entry = &unit.value;
        let at = self.acc.stamp(
            entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_ms),
        );
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            return;
        }
        let Some(message) = entry.get("message") else {
            return;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                let text = message.get("content").map(content_text).unwrap_or_default();
                if let Some(clean) = clean_pi_user_text(&text) {
                    self.acc.close_turn();
                    let id = entry
                        .get("id")
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("o{}", unit.start), str::to_string);
                    self.acc
                        .push(Item::plain(format!("u:{id}"), "user", clean, at));
                    self.acc.running = Some(true);
                }
            }
            Some("assistant") => self.assistant(unit, message, at),
            Some("toolResult") => {
                if let Some(id) = message.get("toolCallId").and_then(Value::as_str) {
                    let is_error = message.get("isError").and_then(Value::as_bool) == Some(true);
                    self.acc.tool_result(
                        id,
                        message.get("content").unwrap_or(&Value::Null),
                        is_error,
                    );
                }
                self.acc.running = Some(true);
            }
            _ => {}
        }
    }

    fn finish(self: Box<Self>, consumed: u64, stale: bool) -> Outcome {
        self.acc.finish(consumed, stale)
    }
}

fn message_text(value: &Value) -> Option<(&str, String)> {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = value.get("message")?;
    let role = message.get("role")?.as_str()?;
    Some((role, content_text(message.get("content")?)))
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

pub(super) fn collect_jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

fn canonical_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

fn modified_at(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(crate::now_ms)
}

fn short_title(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text).trim();
    let clipped: String = first_line
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    if clipped.is_empty() {
        "Pi session".to_string()
    } else {
        clipped
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

#[cfg(test)]
mod reducer_tests {
    use serde_json::json;

    #[test]
    fn a_tool_using_turn_is_one_message_that_ends_on_its_stop_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pi.jsonl");
        let lines = [
            json!({"type":"session","id":"s","cwd":"/w","timestamp":"2026-08-26T12:00:00.000Z"}),
            json!({"type":"message","id":"m1","timestamp":"2026-08-26T12:00:01.000Z",
                "message":{"role":"user","content":[{"type":"text","text":"count files"}]}}),
            json!({"type":"message","id":"m2","timestamp":"2026-08-26T12:00:02.000Z",
                "message":{"role":"assistant","stopReason":"toolUse","content":[
                    {"type":"thinking","thinking":"plan"},
                    {"type":"text","text":"Counting."},
                    {"type":"toolCall","id":"tc1","name":"bash","arguments":{"command":"ls | wc -l"}}]}}),
            json!({"type":"message","id":"m3","timestamp":"2026-08-26T12:00:03.000Z",
                "message":{"role":"toolResult","toolCallId":"tc1","toolName":"bash","isError":false,
                    "content":[{"type":"text","text":"3"}]}}),
        ];
        let write = |n: usize| {
            let text: String = lines[..n].iter().map(|l| format!("{l}\n")).collect();
            std::fs::write(&path, text).unwrap();
            crate::importer::tail::read("pi-cli", "Developer", &path, 0, false)
                .unwrap()
                .unwrap()
        };
        let mid = write(4);
        assert_eq!(mid.items.len(), 1);
        assert!(mid.open.is_some());
        assert_eq!(mid.running, Some(true));

        let mut all = lines.to_vec();
        all.push(json!({"type":"message","id":"m4","timestamp":"2026-08-26T12:00:04.000Z",
            "message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Three."}]}}));
        let text: String = all.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let done = crate::importer::tail::read("pi-cli", "Developer", &path, 0, false)
            .unwrap()
            .unwrap();
        assert_eq!(done.running, Some(false));
        assert!(done.open.is_none());
        let turn = &done.items[1];
        assert_eq!(turn.id, "t:m2");
        assert_eq!(turn.text, "Counting.\n\nThree.");
        let msg = turn.msg.as_ref().unwrap();
        assert_eq!(msg["toolCalls"][0]["rawOutput"], "3");
        assert_eq!(msg["toolCalls"][0]["status"], "completed");
        assert_eq!(msg["thoughtText"], "plan");
    }
}
