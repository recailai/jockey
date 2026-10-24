//! DeepSeek Harness transcripts.
//!
//! dsh keeps each conversation in `~/.dsh/sessions/<encoded cwd>/session-<id>/`, as an
//! append-only event log `session[.vN].jsonl.zstd`: one zstd frame per JSON line. The first
//! line is the session header (`id`, `cwd`, `createdAt`, `isSeeded`, `delegationDepth`); the
//! rest are events (`user/message`, `assistant/message`, `turn/start`, `session/title`, ...).
//! dsh's ACP adapter lists sessions without titles or times and cannot replay them
//! (`session/resume` only), so history comes from here, as for Claude, Codex and Pi.

use super::{DiscoveredSession, ParsedImportedMessage};
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

/// The model's visible reply in one step; reasoning and tool calls are not part of it.
fn assistant_text(line: &Value) -> Option<String> {
    if event_type(line) != "assistant/message" {
        return None;
    }
    non_empty(text_blocks(line.pointer("/data/message/content")?))
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

/// One message per user input and per step that said something. Steps are not merged into
/// one reply per turn: the log is appended to while the session runs, and a message that
/// never changes once written keeps external sync append-only.
pub(crate) fn parse_dsh_session_messages(path: &Path) -> Vec<ParsedImportedMessage> {
    messages(&read_lines(path), modified_at(path))
}

fn messages(lines: &[Value], fallback_time: i64) -> Vec<ParsedImportedMessage> {
    if lines.first().and_then(header).is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut last_time = fallback_time;
    for line in lines {
        let (role_name, content) = if let Some(text) = user_text(line) {
            ("user", text)
        } else if let Some(text) = assistant_text(line) {
            ("Developer", text)
        } else {
            continue;
        };
        // Strictly increasing, so messages written in the same millisecond keep their order.
        let created_at = event_time(line).unwrap_or(last_time).max(last_time + 1);
        last_time = created_at;
        out.push(ParsedImportedMessage {
            role_name: role_name.to_string(),
            content,
            created_at,
        });
    }
    out
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

    fn assistant(blocks: Value, time: i64) -> Value {
        json!({ "type": "assistant/message", "time": time, "data": {
            "message": { "role": "assistant", "content": blocks }
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

    #[test]
    fn messages_keep_user_input_and_visible_replies_in_order() {
        let lines = vec![
            head(json!({})),
            user("runtime-context", "cwd: /w/app", 110),
            user("user", "Fix it", 120),
            assistant(
                json!([{ "type": "reasoning", "text": "hmm" }, { "type": "tool-call" }]),
                130,
            ),
            assistant(json!([{ "type": "text", "text": "Done." }]), 130),
            user("user", "Thanks", 125),
        ];
        let got = messages(&lines, 0);
        let roles: Vec<_> = got
            .iter()
            .map(|m| (m.role_name.as_str(), m.content.as_str()))
            .collect();
        assert_eq!(
            roles,
            [
                ("user", "Fix it"),
                ("Developer", "Done."),
                ("user", "Thanks")
            ]
        );
        assert!(got.windows(2).all(|w| w[0].created_at < w[1].created_at));
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
