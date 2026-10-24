//! Reading provider transcripts incrementally and grouping them into Jockey-shaped items.
//!
//! A transcript is append-only, so a binding remembers a byte offset (the end of the last
//! line it fully handled) plus a small fingerprint of the file around it. A sync reads only
//! the bytes past that offset, drops a trailing partial line, and hands each complete line
//! to a per-provider reducer. Reducers turn the provider's entries into [`Item`]s: user
//! prompts, event rows, and one assistant message per provider turn, built with the
//! engine's own [`Transcript`] so an imported turn has exactly the payload (text, thought,
//! tool-call segments) a turn Jockey ran itself has.
//!
//! A turn that has not reached its end marker is never returned as an item: the reducer
//! reports where the open turn starts (the next sync re-reads from there, which is bounded by
//! the turn's size, not the file's) and a preview message for the live-turn mechanism.

use crate::engine::transcript::{storage_columns, Transcript};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const FINGERPRINT_HEAD: usize = 256;
const FINGERPRINT_TAIL: u64 = 128;
const MAX_TOOL_OUTPUT: usize = 8 * 1024;
const MAX_TOOL_INPUT_STRING: usize = 4 * 1024;

/// One complete line (or zstd frame) of a transcript.
pub(crate) struct Unit {
    pub(crate) start: u64,
    pub(crate) value: Value,
}

/// A message to store: a user prompt, an event row, or a whole assistant turn.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    /// Provider-native id, without the conversation prefix (`u:<uuid>`, `t:<uuid>`, ...).
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) text: String,
    pub(crate) at: i64,
    /// The structured message of an assistant turn, as `Transcript::message` builds it.
    pub(crate) msg: Option<Value>,
}

impl Item {
    pub(crate) fn plain(id: String, role: &str, text: String, at: i64) -> Self {
        Self {
            id,
            role: role.to_string(),
            text,
            at,
            msg: None,
        }
    }

    pub(crate) fn to_row(&self, prefix: &str) -> crate::db::app_session::ImportRow {
        let (content_type, payload) = match &self.msg {
            Some(msg) => storage_columns(msg),
            None => (None, None),
        };
        crate::db::app_session::ImportRow {
            client_id: format!("{prefix}:{}", self.id),
            role_name: self.role.clone(),
            content: self.text.clone(),
            content_type,
            payload,
            created_at: self.at,
        }
    }

    /// The message as clients render it (`message/added`).
    pub(crate) fn to_json(&self, prefix: &str) -> Value {
        let id = format!("{prefix}:{}", self.id);
        match &self.msg {
            Some(msg) => {
                let mut msg = msg.clone();
                msg["id"] = json!(id);
                msg
            }
            None => json!({ "id": id, "roleName": self.role, "text": self.text, "at": self.at }),
        }
    }
}

/// What reading a stretch of transcript produced.
pub(crate) struct Outcome {
    /// Complete items, in transcript order.
    pub(crate) items: Vec<Item>,
    /// Where the next read resumes: the end of everything handled, or the start of the open
    /// turn when one is still running.
    pub(crate) commit: u64,
    /// The turn still in progress, if any (not persisted).
    pub(crate) open: Option<Item>,
    /// Whether the provider is mid-turn according to the last end marker seen in this read;
    /// `None` when the read held no marker at all.
    pub(crate) running: Option<bool>,
}

pub(crate) trait Reduce {
    fn feed(&mut self, unit: &Unit);
    /// `consumed` is the end of the last line read; `stale` means the file has been quiet long
    /// enough that a turn without an end marker is treated as finished.
    fn finish(self: Box<Self>, consumed: u64, stale: bool) -> Outcome;
}

pub(crate) fn reducer_for(
    runtime_kind: &str,
    role_label: &str,
) -> Option<(Box<dyn Reduce>, ReadKind)> {
    use super::{claude, codex, dsh, pi};
    let role = role_label.to_string();
    match runtime_kind {
        "claude-code" | "claude-native" => Some((
            Box::new(claude::ClaudeReducer::new(role)),
            ReadKind::Lines(claude::skip_line),
        )),
        "codex-cli" => Some((
            Box::new(codex::CodexReducer::new(role)),
            ReadKind::Lines(|_| false),
        )),
        "pi-cli" => Some((
            Box::new(pi::PiReducer::new(role)),
            ReadKind::Lines(|_| false),
        )),
        "dsh-cli" => Some((Box::new(dsh::DshReducer::new(role)), ReadKind::Frames)),
        _ => None,
    }
}

/// Read `path` from `from` with the runtime's reducer. `None` for a runtime without a reader.
pub(crate) fn read(
    runtime_kind: &str,
    role_label: &str,
    path: &Path,
    from: u64,
    stale: bool,
) -> io::Result<Option<Outcome>> {
    let Some((mut reducer, kind)) = reducer_for(runtime_kind, role_label) else {
        return Ok(None);
    };
    let consumed = scan(kind, path, from, reducer.as_mut())?;
    Ok(Some(reducer.finish(consumed, stale)))
}

#[derive(Clone, Copy)]
pub(crate) enum ReadKind {
    /// Newline-delimited JSON; the predicate rejects a raw line before it is parsed.
    Lines(fn(&[u8]) -> bool),
    /// One zstd frame per record (dsh).
    Frames,
}

/// Feed every complete record at or after `from` to `reducer`. Returns the offset after the
/// last record handled (a torn tail is left for the next read).
pub(crate) fn scan(
    kind: ReadKind,
    path: &Path,
    from: u64,
    reducer: &mut dyn Reduce,
) -> io::Result<u64> {
    match kind {
        ReadKind::Lines(skip) => scan_lines(path, from, skip, reducer),
        ReadKind::Frames => scan_frames(path, from, reducer),
    }
}

fn scan_lines(
    path: &Path,
    from: u64,
    skip: fn(&[u8]) -> bool,
    reducer: &mut dyn Reduce,
) -> io::Result<u64> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buf = Vec::new();
    let mut pos = from;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        let complete = buf.last() == Some(&b'\n');
        let start = pos;
        let end = pos + n as u64;
        let body = if complete { &buf[..n - 1] } else { &buf[..] };
        if body.iter().all(u8::is_ascii_whitespace) || skip(body) {
            if !complete {
                break;
            }
            pos = end;
            continue;
        }
        match serde_json::from_slice::<Value>(body) {
            Ok(value) => {
                reducer.feed(&Unit { start, value });
                pos = end;
            }
            // A complete but unparsable line is garbage to step over; an unterminated one is
            // a write in progress and is read again once its newline lands.
            Err(_) if complete => pos = end,
            Err(_) => break,
        }
    }
    Ok(pos)
}

fn scan_frames(path: &Path, from: u64, reducer: &mut dyn Reduce) -> io::Result<u64> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let total = bytes.len();
    let mut rest: &[u8] = &bytes;
    let mut text = Vec::new();
    while !rest.is_empty() {
        let before = rest.len();
        text.clear();
        // The decoder advances `rest` even through a frame it then rejects.
        let frame_bytes = &bytes[total - before..];
        let Ok(mut frame) = ruzstd::decoding::StreamingDecoder::new(&mut rest) else {
            rest = frame_bytes;
            break;
        };
        // A frame cut off mid-write can read back as a short success; only a finished frame
        // (last block decoded, checksum seen) is consumed.
        if frame.read_to_end(&mut text).is_err() || !frame.decoder.is_finished() {
            drop(frame);
            rest = frame_bytes;
            break;
        }
        drop(frame);
        let start = from + (total - before) as u64;
        let values: Vec<Option<Value>> = String::from_utf8_lossy(&text)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| serde_json::from_str::<Value>(line).ok())
            .collect();
        // A short last frame can pass for a finished one when its block is stored raw; if the
        // last frame does not hold whole JSON lines it is a write in progress. In the middle
        // of the file the same frame is garbage to step over.
        if rest.is_empty() && values.iter().any(Option::is_none) {
            rest = frame_bytes;
            break;
        }
        for value in values.into_iter().flatten() {
            reducer.feed(&Unit { start, value });
        }
    }
    Ok(from + (total - rest.len()) as u64)
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

/// Identity of the file around `offset`: its first bytes (an append never changes them) and
/// the bytes just before the offset (a rewrite that keeps the length almost never keeps
/// them). Two small reads, whatever the file's size.
pub(crate) fn fingerprint(path: &Path, offset: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    // Only bytes inside the handled region: while the file is shorter than the head window,
    // appends would otherwise change the "head".
    let mut head = vec![0u8; FINGERPRINT_HEAD.min(offset as usize)];
    file.read_exact(&mut head)?;
    let tail_start = offset.saturating_sub(FINGERPRINT_TAIL);
    let mut tail = vec![0u8; (offset - tail_start) as usize];
    file.seek(SeekFrom::Start(tail_start))?;
    file.read_exact(&mut tail)?;
    Ok(format!("{:016x}:{:016x}", fnv(&head), fnv(&tail)))
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// `2026-10-20T00:40:27.123Z` (or with a `+hh:mm` offset) as epoch milliseconds.
pub(crate) fn parse_iso_ms(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let (clock, offset_ms) = match time.find(['Z', '+', '-']) {
        Some(i) if time.as_bytes()[i] == b'Z' => (&time[..i], 0),
        Some(i) => {
            let sign = if time.as_bytes()[i] == b'-' { -1 } else { 1 };
            let (h, mi) = time[i + 1..]
                .split_once(':')
                .unwrap_or((&time[i + 1..], "0"));
            (
                &time[..i],
                sign * (h.parse::<i64>().ok()? * 60 + mi.parse::<i64>().ok()?) * 60_000,
            )
        }
        None => (time, 0),
    };
    let mut c = clock.split(':');
    let h: i64 = c.next()?.parse().ok()?;
    let mi: i64 = c.next()?.parse().ok()?;
    let sec_text = c.next().unwrap_or("0");
    let (sec, frac) = sec_text.split_once('.').unwrap_or((sec_text, "0"));
    let sec: i64 = sec.parse().ok()?;
    let frac_ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    // Days since 1970-01-01 (civil-from-days inverse, proleptic Gregorian).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + frac_ms - offset_ms)
}

// ---------------------------------------------------------------------------
// Turn accumulation
// ---------------------------------------------------------------------------

/// Tool kind (ACP vocabulary) for a provider's native tool name.
pub(crate) fn tool_kind(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "exec" | "execute" | "exec_command" | "run_command" | "terminal"
        | "local_shell" | "local_shell_call" | "command_execution" => "execute",
        "read" | "read_file" | "view" | "view_file" | "cat" | "notebookread" => "read",
        "edit" | "multiedit" | "notebookedit" | "write" | "create_file" | "apply_patch"
        | "str_replace_editor" | "str_replace" | "edit_file" | "patch" => "edit",
        "grep" | "glob" | "ls" | "search" | "find" | "grep_search" | "file_search" | "list" => {
            "search"
        }
        "webfetch" | "websearch" | "fetch" | "web_search" => "fetch",
        "task" | "agent" | "subagent" => "agent",
        "todowrite" | "think" => "think",
        _ => "other",
    }
}

fn first_str<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 120 {
        format!("{}...", line.chars().take(117).collect::<String>())
    } else {
        line.to_string()
    }
}

/// A short row title for a tool call, in the style the ACP adapters use.
pub(crate) fn tool_title(name: &str, input: &Value) -> String {
    let detail = match tool_kind(name) {
        "execute" => first_str(input, &["description", "command", "cmd"]).map(one_line),
        "read" | "edit" => first_str(input, &["file_path", "filePath", "path", "notebook_path"])
            .map(str::to_string),
        "search" => first_str(input, &["pattern", "query", "path"]).map(str::to_string),
        "fetch" => first_str(input, &["url", "query"]).map(str::to_string),
        "agent" => first_str(input, &["description", "prompt"]).map(one_line),
        _ => None,
    };
    match detail {
        Some(d) if tool_kind(name) == "execute" || tool_kind(name) == "agent" => d,
        Some(d) => format!("{name} {d}"),
        None => name.to_string(),
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n...[truncated]", &text[..end])
}

/// Bounds every string inside a tool's input so one `Write` of a large file cannot bloat the
/// stored message.
pub(crate) fn bound_input(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(truncate_chars(s, MAX_TOOL_INPUT_STRING)),
        Value::Array(items) => Value::Array(items.iter().map(bound_input).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), bound_input(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Text of a tool result's content: a string, or the text blocks of an array.
pub(crate) fn output_text(content: &Value) -> String {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                Value::String(s) => Some(s.as_str()),
                _ => b.get("text").and_then(Value::as_str),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    truncate_chars(&text, MAX_TOOL_OUTPUT)
}

/// One assistant turn being assembled.
pub(crate) struct TurnAcc {
    id: String,
    start: u64,
    at: i64,
    tr: Transcript,
    tools: HashSet<String>,
    has_text: bool,
}

impl TurnAcc {
    fn new(id: String, start: u64, at: i64, role: &str) -> Self {
        Self {
            id,
            start,
            at,
            tr: Transcript::new(role),
            tools: HashSet::new(),
            has_text: false,
        }
    }

    fn has_content(&self) -> bool {
        self.tr.has_content()
    }

    fn preview(&self) -> Item {
        self.item(&self.tr.message(&self.id, self.at, None))
    }

    fn item(&self, msg: &Value) -> Item {
        Item {
            id: self.id.clone(),
            role: msg
                .get("roleName")
                .and_then(Value::as_str)
                .unwrap_or("Developer")
                .to_string(),
            text: msg
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            at: self.at,
            msg: Some(msg.clone()),
        }
    }
}

/// The reducers' shared state: finished items, the turn in progress, the running flag.
pub(crate) struct Acc {
    pub(crate) role: String,
    pub(crate) items: Vec<Item>,
    cur: Option<TurnAcc>,
    pub(crate) running: Option<bool>,
    pub(crate) last_at: i64,
}

impl Acc {
    pub(crate) fn new(role: String) -> Self {
        Self {
            role,
            items: Vec::new(),
            cur: None,
            running: None,
            last_at: 0,
        }
    }

    pub(crate) fn has_turn(&self) -> bool {
        self.cur.is_some()
    }

    /// Keeps items in transcript time order and ensures `at` never goes backwards.
    pub(crate) fn stamp(&mut self, at: Option<i64>) -> i64 {
        let at = at.unwrap_or(self.last_at).max(self.last_at);
        self.last_at = at;
        at
    }

    pub(crate) fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    pub(crate) fn open_turn(&mut self, id: String, start: u64, at: i64) {
        if self.cur.is_none() {
            self.cur = Some(TurnAcc::new(id, start, at, &self.role));
        }
    }

    /// Opens a turn with `id` unless one is already open.
    pub(crate) fn ensure_turn(&mut self, id: impl FnOnce() -> String, start: u64, at: i64) {
        if self.cur.is_none() {
            self.open_turn(id(), start, at);
        }
        if let Some(cur) = &mut self.cur {
            cur.at = cur.at.max(at);
        }
    }

    pub(crate) fn text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let Some(cur) = &mut self.cur {
            let separator = if cur.has_text { "\n\n" } else { "" };
            cur.has_text = true;
            cur.tr.apply_text(&format!("{separator}{text}"), None);
        }
    }

    pub(crate) fn thought(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if let Some(cur) = &mut self.cur {
            cur.tr
                .apply_event(&json!({ "kind": "thoughtDelta", "text": text }), None);
        }
    }

    pub(crate) fn tool_call(&mut self, id: &str, name: &str, input: &Value, parent: Option<&str>) {
        let Some(cur) = &mut self.cur else { return };
        if !cur.tools.insert(id.to_string()) {
            return;
        }
        let mut event = json!({
            "kind": "toolCall",
            "toolCallId": id,
            "toolName": name,
            "title": tool_title(name, input),
            "toolKind": tool_kind(name),
            "status": "running",
        });
        if !input.is_null() {
            event["rawInput"] = bound_input(input);
        }
        if let Some(parent) = parent {
            event["parentId"] = json!(parent);
        }
        cur.tr.apply_event(&event, None);
    }

    pub(crate) fn tool_result(&mut self, id: &str, content: &Value, is_error: bool) {
        let Some(cur) = &mut self.cur else { return };
        if !cur.tools.contains(id) {
            return;
        }
        let output = output_text(content);
        let mut event = json!({
            "kind": "toolCallUpdate",
            "toolCallId": id,
            "status": if is_error { "failure" } else { "completed" },
        });
        if !output.is_empty() {
            event["rawOutput"] = json!(output);
        }
        cur.tr.apply_event(&event, None);
    }

    /// End the turn in progress and turn it into an item. Tools that never reported a result
    /// are closed as interrupted instead of left spinning.
    pub(crate) fn close_turn(&mut self) {
        let Some(mut cur) = self.cur.take() else {
            return;
        };
        if !cur.has_content() {
            return;
        }
        cur.tr.settle_open_tools("interrupted");
        let msg = cur.tr.message(&cur.id, cur.at, None);
        let item = cur.item(&msg);
        self.items.push(item);
    }

    pub(crate) fn finish(mut self, consumed: u64, close_open: bool) -> Outcome {
        let mut commit = consumed;
        let mut open = None;
        if close_open {
            self.close_turn();
        } else if let Some(cur) = self.cur.take() {
            commit = cur.start;
            if cur.has_content() {
                open = Some(cur.preview());
            }
        }
        Outcome {
            items: self.items,
            commit,
            open,
            running: self.running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps_parse_to_epoch_milliseconds() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-10-20T00:40:27.123Z"),
            Some(1_792_456_827_123)
        );
        assert_eq!(
            parse_iso_ms("2026-10-20T08:40:27+08:00"),
            Some(1_792_456_827_000)
        );
        assert_eq!(
            parse_iso_ms("2024-02-29T12:00:00.5Z"),
            Some(1_709_208_000_500)
        );
        assert_eq!(parse_iso_ms("nonsense"), None);
    }

    #[test]
    fn fingerprint_tracks_the_bytes_before_the_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n").unwrap();
        let before = fingerprint(&path, 8).unwrap();
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        assert_eq!(fingerprint(&path, 8).unwrap(), before, "appends keep it");
        std::fs::write(&path, "{\"a\":9}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        assert_ne!(
            fingerprint(&path, 8).unwrap(),
            before,
            "a rewrite changes it"
        );
    }

    struct Collect(Vec<(u64, String)>);
    impl Reduce for Collect {
        fn feed(&mut self, unit: &Unit) {
            self.0.push((unit.start, unit.value.to_string()));
        }
        fn finish(self: Box<Self>, consumed: u64, _stale: bool) -> Outcome {
            Outcome {
                items: Vec::new(),
                commit: consumed,
                open: None,
                running: None,
            }
        }
    }

    #[test]
    fn lines_are_read_from_an_offset_and_a_partial_tail_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{\"a\":1}\nnot json\n{\"b\":2}\n{\"c\":").unwrap();
        let mut got = Collect(Vec::new());
        let end = scan_lines(&path, 0, |_| false, &mut got).unwrap();
        assert_eq!(
            got.0.len(),
            2,
            "garbage line skipped, partial tail not read"
        );
        assert_eq!(end, 8 + 9 + 8, "offset stops before the partial line");
        // The writer finishes the line; reading resumes exactly there.
        std::fs::write(&path, "{\"a\":1}\nnot json\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        let mut more = Collect(Vec::new());
        let end2 = scan_lines(&path, end, |_| false, &mut more).unwrap();
        assert_eq!(more.0.len(), 1);
        assert_eq!(more.0[0].1, "{\"c\":3}");
        assert_eq!(end2, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn a_complete_final_line_without_newline_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}").unwrap();
        let mut got = Collect(Vec::new());
        let end = scan_lines(&path, 0, |_| false, &mut got).unwrap();
        assert_eq!(got.0.len(), 2);
        assert_eq!(end, 15);
    }

    /// `JOCKEY_SMOKE=<runtime>:<path> cargo test smoke -- --ignored --nocapture`: shapes only.
    #[test]
    #[ignore]
    fn smoke_real_transcript() {
        let Ok(spec) = std::env::var("JOCKEY_SMOKE") else {
            return;
        };
        let (runtime, path) = spec.split_once(':').unwrap();
        let started = std::time::Instant::now();
        let out = read(runtime, "Developer", Path::new(path), 0, true)
            .unwrap()
            .unwrap();
        let tools: usize = out
            .items
            .iter()
            .filter_map(|i| i.msg.as_ref())
            .map(|m| m["toolCalls"].as_array().map_or(0, Vec::len))
            .sum();
        let users = out.items.iter().filter(|i| i.role == "user").count();
        let turns = out.items.iter().filter(|i| i.msg.is_some()).count();
        println!(
            "items={} users={users} turns={turns} tools={tools} running={:?} commit={} in {:?}",
            out.items.len(),
            out.running,
            out.commit,
            started.elapsed()
        );
    }
}
