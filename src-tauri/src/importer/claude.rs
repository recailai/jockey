use super::{DiscoveredSession, ParsedImportedMessage};
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

pub(crate) fn scan_claude_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return Vec::new();
    }
    let config_dir =
        std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| format!("{}/.claude", home));
    let encoded = encode_claude_project_dir(project_path);
    let project_dir = Path::new(&config_dir).join("projects").join(&encoded);
    if !project_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let session_id = file_stem.to_string();
                    let Some((title, turn_count)) = inspect_claude_session(&path, &session_id) else {
                        continue;
                    };
                    let metadata = entry.metadata().ok();
                    let mtime = metadata
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or_else(crate::now_ms);

                    sessions.push(DiscoveredSession {
                        runtime_kind: "claude-native",
                        agent: "Claude Code",
                        session_id,
                        file_path: path,
                        cwd: canonical_project_path(project_path),
                        title,
                        created_at: mtime,
                        last_active_at: mtime,
                        turn_count,
                        parse_messages: parse_claude_session_messages,
                    });
                }
            }
        }
    }
    sessions.sort_by_key(|b| std::cmp::Reverse(b.last_active_at));
    sessions
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

fn clean_claude_user_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
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

        let has_ai_title = line.contains("\"aiTitle\"") || line.contains("\"agentName\"") || line.contains("\"customTitle\"");
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

pub(crate) fn parse_claude_session_messages(path: &Path) -> Vec<ParsedImportedMessage> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let base_time = crate::now_ms();
    let mut offset = 0i64;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        if val.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
            || val.get("isCompactSummary").and_then(|m| m.as_bool()) == Some(true)
        {
            continue;
        }

        let entry_type = val.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        offset += 50;
        let timestamp_ms = base_time + offset;

        if entry_type == "user" {
            if let Some(msg) = val.get("message") {
                let mut user_text = String::new();
                if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                    user_text.push_str(content_str);
                } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                    for block in content_arr {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(txt) = block.get("text").and_then(|t| t.as_str()) {
                                user_text.push_str(txt);
                            }
                        }
                    }
                }

                if let Some(cleaned) = clean_claude_user_text(&user_text) {
                    messages.push(ParsedImportedMessage {
                        role_name: "user".to_string(),
                        content: cleaned,
                        created_at: timestamp_ms,
                    });
                }
            }
        } else if entry_type == "assistant" {
            if let Some(msg) = val.get("message") {
                let mut assistant_parts = Vec::new();
                if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                    assistant_parts.push(content_str.to_string());
                } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                    for block in content_arr {
                        let block_type = block
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or_default();
                        if block_type == "text" {
                            if let Some(txt) = block.get("text").and_then(|t| t.as_str()) {
                                let trimmed = txt.trim();
                                if !trimmed.is_empty() {
                                    assistant_parts.push(trimmed.to_string());
                                }
                            }
                        } else if block_type == "tool_use" {
                            let tool_name =
                                block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                            let command_hint = block
                                .pointer("/input/command")
                                .and_then(|c| c.as_str())
                                .or_else(|| {
                                    block.pointer("/input/description").and_then(|c| c.as_str())
                                })
                                .unwrap_or("");
                            if !command_hint.is_empty() {
                                assistant_parts.push(format!("`{tool_name}`: {command_hint}"));
                            }
                        }
                    }
                }

                let assistant_text = assistant_parts.join("\n\n").trim().to_string();
                if !assistant_text.is_empty() {
                    messages.push(ParsedImportedMessage {
                        role_name: "Developer".to_string(),
                        content: assistant_text,
                        created_at: timestamp_ms,
                    });
                }
            }
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let sessions = scan_claude_sessions("/Users/sexy/Documents/GitHub/arise-ops");
        println!("Scanned sessions: {:?}", sessions);
        if let Some(first) = sessions.first() {
            let msgs = parse_claude_session_messages(&first.file_path);
            println!("Parsed messages count: {}", msgs.len());
            assert!(!msgs.is_empty(), "Should parse messages from session file");
        }
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
