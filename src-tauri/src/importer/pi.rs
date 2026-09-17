use super::{DiscoveredSession, ParsedImportedMessage};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub(crate) fn scan_pi_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let root = PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("sessions");
    let project_cwd = canonical_path(project_path);
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);
    files
        .into_iter()
        .filter_map(|file_path| scan_file(&file_path, &project_cwd))
        .collect()
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

fn scan_file(path: &Path, project_cwd: &str) -> Option<DiscoveredSession> {
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
    if cwd != project_cwd {
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
        parse_messages: parse_pi_session_messages,
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

pub(crate) fn parse_pi_session_messages(path: &Path) -> Vec<ParsedImportedMessage> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    let mut created_at = modified_at(path);
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some((role, text)) = message_text(&value) else {
            continue;
        };
        let role_name = match role {
            "user" => "user",
            "assistant" => "Developer",
            _ => continue,
        };
        let final_text = if role == "user" {
            let Some(clean) = clean_pi_user_text(&text) else {
                continue;
            };
            clean
        } else {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            trimmed
        };
        created_at += 50;
        messages.push(ParsedImportedMessage {
            role_name: role_name.to_string(),
            content: final_text,
            created_at,
        });
    }
    messages
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

fn collect_jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
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
