//! DeepSeek Harness transcripts.
//!
//! dsh keeps each conversation in `~/.dsh/sessions/<encoded cwd>/session-<id>/`, as an
//! append-only event log `session[.vN].jsonl.zstd`: one zstd frame per JSON line. The first
//! line is the session header (`id`, `cwd`, `createdAt`, `isSeeded`, `delegationDepth`); the
//! rest are events (`user/message`, `assistant/message`, `turn/start`, `session/title`, ...).
//! dsh's ACP adapter lists sessions without titles or times and cannot replay them
//! (`session/resume` only), so history comes from here, as for Claude, Codex and Pi.

use super::tail::{Acc, Item, Outcome, Reduce, Unit};
use super::DiscoveredSession;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

const RUNTIME: &str = "dsh-cli";
const AGENT: &str = "DeepSeek Harness";

pub(crate) fn scan_all_dsh_sessions() -> Vec<DiscoveredSession> {
    scan(None)
}

pub(crate) fn scan_dsh_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    scan(Some(&canonical_path(project_path)))
}

fn scan(project_cwd: Option<&str>) -> Vec<DiscoveredSession> {
    let Some(root) = sessions_root() else {
        return Vec::new();
    };
    session_dirs(&root)
        .into_iter()
        .filter_map(|dir| transcript_in(&dir))
        .filter_map(|file| scan_file(&file, project_cwd))
        .collect()
}

pub(crate) fn sessions_root() -> Option<PathBuf> {
    std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".dsh")))
        .map(|home| home.join("sessions"))
}

fn session_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|cwd_dir| std::fs::read_dir(cwd_dir.path()).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// The newest-format log in a session directory. An upgraded dsh rewrites the session under a
/// higher version and leaves the old file behind.
fn transcript_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let version = log_version(&name)?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

/// `session.jsonl.zstd` → 0, `session.v4.jsonl.zstd` → 4; anything else is not a log.
fn log_version(name: &str) -> Option<u32> {
    let middle = name.strip_prefix("session")?.strip_suffix(".jsonl.zstd")?;
    if middle.is_empty() {
        return Some(0);
    }
    middle.strip_prefix(".v")?.parse().ok()
}

/// Where conversation `provider_id` (`session-<uuid>`) is logged, if it is on disk.
pub(crate) fn locate(provider_id: &str) -> Option<PathBuf> {
    if provider_id.is_empty() || provider_id.contains(['/', '\\']) {
        return None;
    }
    std::fs::read_dir(sessions_root()?)
        .ok()?
        .flatten()
        .map(|cwd_dir| cwd_dir.path().join(provider_id))
        .find(|dir| dir.is_dir())
        .and_then(|dir| transcript_in(&dir))
}

/// The log's JSON lines. A live session may end in a frame still being written; everything
/// before it is kept.
fn read_lines(path: &Path) -> Vec<Value> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let mut rest: &[u8] = &bytes;
    let mut text = Vec::new();
    while !rest.is_empty() {
        let Ok(mut frame) = ruzstd::decoding::StreamingDecoder::new(&mut rest) else {
            break;
        };
        if frame.read_to_end(&mut text).is_err() {
            break;
        }
    }
    String::from_utf8_lossy(&text)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

struct Header {
    id: String,
    cwd: String,
    created_at: Option<i64>,
}

/// The session header, unless the session is not one the user had: a sub-agent's own
/// conversation (`delegationDepth` > 0) or a session seeded by another one.
fn header(line: &Value) -> Option<Header> {
    if line.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let delegated = line
        .get("delegationDepth")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0;
    let seeded = line
        .get("isSeeded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if delegated || seeded {
        return None;
    }
    Some(Header {
        id: line.get("id")?.as_str()?.to_string(),
        cwd: canonical_path(line.get("cwd")?.as_str()?),
        created_at: line.get("createdAt").and_then(Value::as_i64),
    })
}

fn event_type(line: &Value) -> &str {
    line.get("type").and_then(Value::as_str).unwrap_or("")
}

fn event_time(line: &Value) -> Option<i64> {
    line.get("time").and_then(Value::as_i64)
}

/// What the user typed. dsh also records context it injects (time, instructions, skill
/// catalog, tool-job notes) as `user/message`; only `source.kind == "user"` is the user.
fn user_text(line: &Value) -> Option<String> {
    if event_type(line) != "user/message" {
        return None;
    }
    let data = line.get("data")?;
    if data.pointer("/source/kind").and_then(Value::as_str) != Some("user") {
        return None;
    }
    non_empty(text_blocks(data.get("content")?))
}

fn text_blocks(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

fn scan_file(path: &Path, project_cwd: Option<&str>) -> Option<DiscoveredSession> {
    summarize(&read_lines(path), path, project_cwd)
}

fn summarize(lines: &[Value], path: &Path, project_cwd: Option<&str>) -> Option<DiscoveredSession> {
    let header = header(lines.first()?)?;
    // Sessions started in a sub-directory of the project belong to it too.
    if project_cwd.is_some_and(|root| !Path::new(&header.cwd).starts_with(root)) {
        return None;
    }
    let turn_count = lines
        .iter()
        .filter(|line| event_type(line) == "turn/start")
        .count();
    if turn_count == 0 {
        return None;
    }
    // dsh titles a session itself once it has enough to go on; the latest title wins.
    let title = lines
        .iter()
        .rev()
        .filter(|line| event_type(line) == "session/title")
        .find_map(|line| line.pointer("/data/title")?.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(short_title)
        .or_else(|| lines.iter().find_map(user_text).map(|t| short_title(&t)))
        .unwrap_or_else(|| format!("{AGENT} · {}", short_id(&header.id)));
    let last_active_at = lines
        .iter()
        .rev()
        .find_map(event_time)
        .unwrap_or_else(|| modified_at(path));
    Some(DiscoveredSession {
        runtime_kind: RUNTIME,
        agent: AGENT,
        session_id: header.id,
        file_path: path.to_path_buf(),
        cwd: header.cwd,
        title,
        created_at: header.created_at.unwrap_or(last_active_at),
        last_active_at,
        turn_count,
    })
}

/// Groups dsh events into Jockey-shaped items: `turn/start` .. `turn/end` is one assistant
/// message holding the turn's visible text, reasoning and tool calls; user input and the
/// agent's inbox are separate rows. Each record is its own zstd frame, so a sync resumes at a
/// frame boundary. The streaming `*-chunks` events are ignored: `assistant/message` carries the
/// finished step.
pub(crate) struct DshReducer {
    acc: Acc,
}

impl DshReducer {
    pub(crate) fn new(role: String) -> Self {
        Self {
            acc: Acc::new(role),
        }
    }

    fn assistant(&mut self, data: &Value) {
        let Some(blocks) = data.pointer("/message/content").and_then(Value::as_array) else {
            return;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        self.acc.text(text);
                    }
                }
                Some("reasoning") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        self.acc.thought(text);
                    }
                }
                Some("tool-call") => self.tool_call(block, "id", "arguments"),
                _ => {}
            }
        }
    }

    fn tool_call(&mut self, block: &Value, id_key: &str, args_key: &str) {
        let Some(id) = block.get(id_key).and_then(Value::as_str) else {
            return;
        };
        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
        let input = match block.get(args_key) {
            Some(Value::String(args)) => {
                serde_json::from_str(args).unwrap_or_else(|_| Value::String(args.clone()))
            }
            Some(other) => other.clone(),
            None => Value::Null,
        };
        self.acc.tool_call(id, name, &input, None);
    }
}

impl Reduce for DshReducer {
    fn feed(&mut self, unit: &Unit) {
        let line = &unit.value;
        let at = self.acc.stamp(event_time(line));
        let seq = line
            .get("seq")
            .and_then(Value::as_u64)
            .map_or_else(|| format!("o{}", unit.start), |n| n.to_string());
        let data = line.get("data").unwrap_or(&Value::Null);
        match event_type(line) {
            "turn/start" => {
                self.acc.close_turn();
                self.acc.open_turn(format!("t:{seq}"), unit.start, at);
                self.acc.running = Some(true);
            }
            "turn/end" => {
                self.acc.close_turn();
                self.acc.running = Some(false);
            }
            "user/message" => {
                if let Some(text) = user_text(line) {
                    self.acc
                        .push(Item::plain(format!("u:{seq}"), "user", text, at));
                }
            }
            "assistant/message" => {
                self.acc.ensure_turn(|| format!("t:{seq}"), unit.start, at);
                self.assistant(data);
            }
            "tool/call" => {
                if self.acc.has_turn() {
                    self.tool_call(data, "callId", "arguments");
                }
            }
            "tool/result" => {
                if let Some(blocks) = data.pointer("/message/content").and_then(Value::as_array) {
                    for block in blocks {
                        let Some(id) = block.get("toolCallId").and_then(Value::as_str) else {
                            continue;
                        };
                        let is_error = block.get("isError").and_then(Value::as_bool) == Some(true);
                        self.acc.tool_result(
                            id,
                            block.get("content").unwrap_or(&Value::Null),
                            is_error,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(self: Box<Self>, consumed: u64, stale: bool) -> Outcome {
        self.acc.finish(consumed, stale)
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
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or_else(crate::now_ms)
}

fn short_title(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text).trim();
    first_line
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect()
}

/// `session-52f90df4-…` → `52f90df4`.
fn short_id(id: &str) -> &str {
    let id = id.strip_prefix("session-").unwrap_or(id);
    &id[..id.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn head(extra: Value) -> Value {
        let mut h = json!({
            "type": "session", "version": 4, "id": "session-52f90df4-a862",
            "createdAt": 100, "cwd": "/w/app", "isSeeded": false, "delegationDepth": 0
        });
        h.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        h
    }

    fn user(kind: &str, text: &str, time: i64) -> Value {
        json!({ "type": "user/message", "time": time, "data": {
            "role": "user", "source": { "kind": kind },
            "content": [{ "type": "text", "text": text }]
        }})
    }

    fn turn() -> Value {
        json!({ "type": "turn/start", "time": 1, "data": { "turn": 1 } })
    }

    #[test]
    fn log_versions_prefer_the_newest_format() {
        assert_eq!(log_version("session.jsonl.zstd"), Some(0));
        assert_eq!(log_version("session.v4.jsonl.zstd"), Some(4));
        assert_eq!(log_version("session.lock"), None);
        assert_eq!(log_version("session.vx.jsonl.zstd"), None);
    }

    #[test]
    fn summary_counts_turns_titles_and_skips_unowned_sessions() {
        let lines = vec![
            head(json!({})),
            turn(),
            user("time-context", "It is Monday", 110),
            user("user", "Fix the login bug\nmore", 120),
            turn(),
            json!({ "type": "turn/end", "time": 300 }),
        ];
        let s = summarize(&lines, Path::new("/x"), None).unwrap();
        assert_eq!(s.session_id, "session-52f90df4-a862");
        assert_eq!(s.turn_count, 2);
        assert_eq!(s.title, "Fix the login bug");
        assert_eq!(s.created_at, 100);
        assert_eq!(s.last_active_at, 300);
        assert!(summarize(&lines, Path::new("/x"), Some("/elsewhere")).is_none());

        let mut titled = lines.clone();
        titled.push(
            json!({ "type": "session/title", "time": 310, "data": { "title": "Login fix" } }),
        );
        assert_eq!(
            summarize(&titled, Path::new("/x"), None).unwrap().title,
            "Login fix"
        );

        let mut child = lines.clone();
        child[0] = head(json!({ "delegationDepth": 1 }));
        assert!(summarize(&child, Path::new("/x"), None).is_none());
        let mut seeded = lines.clone();
        seeded[0] = head(json!({ "isSeeded": true }));
        assert!(summarize(&seeded, Path::new("/x"), None).is_none());
        assert!(summarize(&[head(json!({}))], Path::new("/x"), None).is_none());
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0x20, payload.len() as u8];
        let size = (payload.len() as u32) << 3 | 1; // last block, raw
        f.extend_from_slice(&size.to_le_bytes()[..3]);
        f.extend_from_slice(payload);
        f
    }

    fn frames(lines: &[Value]) -> Vec<Vec<u8>> {
        lines
            .iter()
            .map(|l| frame(format!("{l}\n").as_bytes()))
            .collect()
    }

    fn event(seq: u64, kind: &str, time: i64, data: Value) -> Value {
        json!({ "type": kind, "seq": seq, "time": time, "data": data })
    }

    #[test]
    fn turns_group_into_one_message_and_sync_resumes_per_frame() {
        let lines = vec![
            head(json!({})),
            event(1, "turn/start", 100, json!({ "turn": 1 })),
            event(
                2,
                "user/message",
                110,
                json!({ "role": "user", "source": { "kind": "runtime-context" },
                "content": [{ "type": "text", "text": "cwd" }] }),
            ),
            event(
                3,
                "user/message",
                120,
                json!({ "role": "user", "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "Fix it" }] }),
            ),
            event(
                4,
                "assistant/message",
                130,
                json!({ "message": { "content": [
                { "type": "reasoning", "text": "hmm" },
                { "type": "text", "text": "Checking." },
                { "type": "tool-call", "id": "c1", "name": "bash", "arguments": { "command": "ls" } }] } }),
            ),
            event(5, "assistant/chunk", 131, json!({})),
            event(
                6,
                "tool/result",
                140,
                json!({ "message": { "content": [
                { "type": "tool-result", "toolCallId": "c1", "content": "a.txt", "isError": false }] } }),
            ),
        ];
        let mut bytes: Vec<u8> = frames(&lines).concat();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.v4.jsonl.zstd");
        std::fs::write(&path, &bytes).unwrap();

        let read = |from: u64| {
            crate::importer::tail::read("dsh-cli", "Developer", &path, from, false)
                .unwrap()
                .unwrap()
        };
        let out = read(0);
        assert_eq!(out.items.len(), 1, "only the user's own input is a row");
        assert_eq!(out.items[0].text, "Fix it");
        assert_eq!(out.running, Some(true));
        let open = out.open.as_ref().expect("turn still open");
        assert_eq!(open.text, "Checking.");
        assert_eq!(
            out.commit,
            frames(&lines[..1]).concat().len() as u64,
            "resumes at the turn start"
        );

        // The turn ends, and a torn frame follows: only whole frames are consumed.
        let tail = frames(&[
            event(
                7,
                "assistant/message",
                150,
                json!({ "message": { "content": [{ "type": "text", "text": "Done." }] } }),
            ),
            event(
                8,
                "turn/end",
                160,
                json!({ "turn": 1, "reason": { "kind": "completed" } }),
            ),
        ]);
        bytes.extend(tail.concat());
        let mut torn = frame(b"{\"type\":\"turn/start\"}\n");
        torn.truncate(torn.len() - 3);
        let whole = bytes.len() as u64;
        bytes.extend(torn);
        std::fs::write(&path, &bytes).unwrap();
        let out = read(frames(&lines[..1]).concat().len() as u64);
        assert!(out.open.is_none());
        assert_eq!(out.running, Some(false));
        let turn = out.items.iter().find(|i| i.msg.is_some()).unwrap();
        assert_eq!(turn.text, "Checking.\n\nDone.");
        let msg = turn.msg.as_ref().unwrap();
        assert_eq!(msg["toolCalls"][0]["status"], "completed");
        assert_eq!(msg["toolCalls"][0]["rawOutput"], "a.txt");
        assert_eq!(out.commit, whole);
    }

    #[test]
    fn multi_frame_logs_decode_up_to_a_torn_tail() {
        // Two complete zstd frames (raw blocks) followed by a truncated third.
        fn frame(payload: &[u8]) -> Vec<u8> {
            let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0x20, payload.len() as u8];
            let size = (payload.len() as u32) << 3 | 1; // last block, raw
            f.extend_from_slice(&size.to_le_bytes()[..3]);
            f.extend_from_slice(payload);
            f
        }
        let mut bytes = frame(b"{\"type\":\"session\",\"id\":\"s\",\"cwd\":\"/w\"}\n");
        bytes.extend(frame(b"{\"type\":\"turn/start\"}\n"));
        let mut torn = frame(b"{\"type\":\"turn/start\"}\n");
        torn.truncate(torn.len() - 4);
        bytes.extend(torn);
        let dir = std::env::temp_dir().join(format!("jockey-dsh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.v4.jsonl.zstd");
        std::fs::write(&path, bytes).unwrap();
        let lines = read_lines(&path);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["type"], "turn/start");
    }
}
