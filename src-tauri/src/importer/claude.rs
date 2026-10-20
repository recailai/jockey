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
    let project_root = canonical_project_path(project_path);
    let projects_dir = Path::new(&config_dir).join("projects");

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
                match transcript_cwd(&path) {
                    Some(cwd) if Path::new(&cwd).starts_with(&project_root) => cwd,
                    _ => continue,
                }
            } else {
                project_root.clone()
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
                runtime_kind: "claude-native",
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
    let flagged = entry.pointer("/origin/kind").and_then(|k| k.as_str()) == Some("task-notification");
    let trimmed = text.trim();
    if !flagged && !trimmed.starts_with("<task-notification>") {
        return None;
    }
    let status = tag_body(trimmed, "status").filter(|s| !s.is_empty()).unwrap_or("finished");
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

                if let Some(summary) = task_notification_event(&val, &user_text) {
                    messages.push(ParsedImportedMessage {
                        role_name: "event".to_string(),
                        content: summary,
                        created_at: timestamp_ms,
                    });
                } else if let Some(cleaned) = clean_claude_user_text(&user_text) {
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
    fn task_notifications_become_event_rows_not_user_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sess.jsonl");
        let note = "<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>\n<summary>Agent &quot;x&quot; done</summary>\n</task-notification>";
        let flagged = serde_json::json!({"type":"user","origin":{"kind":"task-notification"},
            "message":{"role":"user","content":note}});
        let bare = serde_json::json!({"type":"user","message":{"role":"user","content":note}});
        let human = serde_json::json!({"type":"user","origin":{"kind":"human"},
            "message":{"role":"user","content":"go on"}});
        std::fs::write(&path, format!("{flagged}\n{bare}\n{human}\n")).expect("write fixture");
        let msgs = parse_claude_session_messages(&path);
        let rows: Vec<(&str, &str)> = msgs
            .iter()
            .map(|m| (m.role_name.as_str(), m.content.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("event", "Background task completed: Agent \"x\" done"),
                ("event", "Background task completed: Agent \"x\" done"),
                ("user", "go on"),
            ]
        );
        assert!(clean_claude_user_text(note).is_none());
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
}
