use super::tail::{parse_iso_ms, tool_kind, Acc, Item, Outcome, Reduce, Unit};
use super::DiscoveredSession;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MAX_METADATA_LINES: usize = 200;

pub(crate) fn scan_all_codex_sessions() -> Vec<DiscoveredSession> {
    sessions_root()
        .map(|root| scan_codex_root(&root, None))
        .unwrap_or_default()
}

pub(crate) fn scan_codex_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    sessions_root()
        .map(|root| scan_codex_root(&root, Some(&canonical_path(project_path))))
        .unwrap_or_default()
}

fn scan_codex_root(root: &Path, project_cwd: Option<&str>) -> Vec<DiscoveredSession> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    files
        .into_iter()
        .filter_map(|file_path| scan_file(&file_path, project_cwd))
        .collect()
}

fn sessions_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".codex").join("sessions"))
}

/// Codex injects context as user items wrapped in one tag (`<environment_context>…`,
/// `<codex_internal_context …>…`, `<external_codex_apps_open_page>…`); new kinds keep
/// appearing, so any message that is a single tag-wrapped block is treated as injected.
fn is_injected_block(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_lowercase() || *c == '_' || *c == '-')
        .collect();
    !name.is_empty() && text.trim_end().ends_with(&format!("</{name}>"))
}

fn clean_codex_user_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || is_injected_block(trimmed) {
        return None;
    }
    // Attachments are listed first; the user's own words follow this heading.
    if trimmed.starts_with("# Files mentioned by the user:") {
        return trimmed
            .split_once("## My request for Codex:")
            .map(|(_, request)| request.trim().to_string())
            .filter(|request| !request.is_empty());
    }
    if trimmed.starts_with("<recommended_plugins>")
        || trimmed.starts_with("# AGENTS.md")
        || trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("<context>")
        || trimmed.starts_with("<codex_internal_context")
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
        || after_context.starts_with("<recommended_plugins>")
        || after_context.starts_with("# AGENTS.md")
        || after_context.starts_with("<environment_context>")
        || after_context.starts_with("<context>")
    {
        return None;
    }
    Some(after_context.to_string())
}

fn scan_file(path: &Path, project_cwd: Option<&str>) -> Option<DiscoveredSession> {
    let file = File::open(path).ok()?;
    let mut session_id = None;
    let mut cwd = None;
    let mut forked = false;
    let mut title = None;
    let mut turn_count = 0;

    for line in BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .take(MAX_METADATA_LINES)
    {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = value.get("payload")?;
            // Sub-agent rollouts (the guardian that reviews approvals, spawned agents) are not
            // conversations the user can resume; they share the parent's `session_id`, so
            // listing them made each one replace its parent, and a fork vanished behind them.
            if payload
                .get("source")
                .and_then(|s| s.get("subagent"))
                .is_some()
            {
                return None;
            }
            // `id` is the thread itself; `session_id` names the conversation group it belongs to.
            session_id = payload
                .get("id")
                .or_else(|| payload.get("session_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            forked = payload
                .get("forked_from_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty());
            cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(canonical_path);
            continue;
        }
        if let Some((role, text)) = response_text(&value) {
            if role == "user" {
                if let Some(clean) = clean_codex_user_text(&text) {
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
    if project_cwd.is_some_and(|root| !super::cwd_in_project(&cwd, root)) {
        return None;
    }
    let turns = count_codex_session_turns(path).max(turn_count);
    if turns == 0 {
        return None;
    }
    let session_id = session_id.or_else(|| path.file_stem()?.to_str().map(ToString::to_string))?;
    let last_active_at = modified_at(path);
    Some(DiscoveredSession {
        runtime_kind: "codex-cli",
        agent: "Codex",
        session_id: session_id.clone(),
        file_path: path.to_path_buf(),
        cwd,
        title: match (title, forked) {
            (Some(t), true) => format!("[Fork] {t}"),
            (Some(t), false) => t,
            (None, true) => format!("Codex_fork_{}", short_id(&session_id)),
            (None, false) => format!("Codex_{}", short_id(&session_id)),
        },
        created_at: last_active_at,
        last_active_at,
        turn_count: turns,
    })
}

fn count_codex_session_turns(path: &Path) -> usize {
    let Ok(file) = File::open(path) else {
        return 0;
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|value| {
            let (role, text) = response_text(&value)?;
            if role == "user" {
                clean_codex_user_text(&text).map(|_| "user")
            } else if role == "assistant" && !text.trim().is_empty() {
                Some("assistant")
            } else {
                None
            }
        })
        .count()
}

/// Groups Codex rollout items into Jockey-shaped items. `task_started` / `task_complete` /
/// `turn_aborted` delimit a turn; rollouts from before those markers existed are closed at the
/// end of what was read, and a user message ends a marker-less turn.
pub(crate) struct CodexReducer {
    acc: Acc,
    markers: bool,
}

impl CodexReducer {
    pub(crate) fn new(role: String) -> Self {
        Self {
            acc: Acc::new(role),
            markers: false,
        }
    }

    fn response_item(&mut self, unit: &Unit, payload: &Value, at: i64) {
        // Older rollouts carry no `type` on message items.
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or(
            if payload.get("role").is_some() {
                "message"
            } else {
                ""
            },
        );
        let lazy_turn = |acc: &mut Acc| {
            acc.ensure_turn(|| format!("t:o{}", unit.start), unit.start, at);
        };
        match kind {
            "message" => {
                let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                let text = payload.get("content").map(content_text).unwrap_or_default();
                match role {
                    "user" => {
                        let Some(clean) = clean_codex_user_text(&text) else {
                            return;
                        };
                        if !self.markers {
                            self.acc.close_turn();
                        }
                        let id = unit
                            .value
                            .get("ordinal")
                            .and_then(Value::as_u64)
                            .map_or_else(|| format!("o{}", unit.start), |n| n.to_string());
                        self.acc
                            .push(Item::plain(format!("u:{id}"), "user", clean, at));
                    }
                    "assistant" => {
                        lazy_turn(&mut self.acc);
                        self.acc.text(&text);
                    }
                    _ => {}
                }
            }
            "reasoning" => {
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                if !summary.trim().is_empty() {
                    lazy_turn(&mut self.acc);
                    self.acc.thought(&summary);
                }
            }
            "function_call" | "custom_tool_call" | "local_shell_call" => {
                let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                    return;
                };
                let name = payload.get("name").and_then(Value::as_str).unwrap_or(
                    if kind == "local_shell_call" {
                        "shell"
                    } else {
                        "tool"
                    },
                );
                let input = match payload.get("arguments").and_then(Value::as_str) {
                    Some(args) => {
                        serde_json::from_str(args).unwrap_or_else(|_| json!({ "input": args }))
                    }
                    None => match payload.get("input").and_then(Value::as_str) {
                        Some(code) if tool_kind(name) == "execute" => json!({ "command": code }),
                        Some(code) => json!({ "input": code }),
                        None => payload.get("action").cloned().unwrap_or(Value::Null),
                    },
                };
                lazy_turn(&mut self.acc);
                self.acc.tool_call(call_id, name, &input, None);
            }
            "function_call_output" | "custom_tool_call_output" => {
                if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                    self.acc.tool_result(
                        call_id,
                        payload.get("output").unwrap_or(&Value::Null),
                        false,
                    );
                }
            }
            _ => {}
        }
    }
}

impl Reduce for CodexReducer {
    fn feed(&mut self, unit: &Unit) {
        let at = self.acc.stamp(
            unit.value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_ms),
        );
        let Some(payload) = unit.value.get("payload") else {
            return;
        };
        match unit.value.get("type").and_then(Value::as_str) {
            Some("event_msg") => match payload.get("type").and_then(Value::as_str) {
                Some("task_started") => {
                    self.markers = true;
                    self.acc.close_turn();
                    let id = payload
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("o{}", unit.start), str::to_string);
                    self.acc.open_turn(format!("t:{id}"), unit.start, at);
                    self.acc.running = Some(true);
                }
                Some("task_complete") => {
                    self.markers = true;
                    self.acc.close_turn();
                    self.acc.running = Some(false);
                }
                Some("turn_aborted") => {
                    self.markers = true;
                    self.acc.close_turn();
                    self.acc.running = Some(false);
                }
                _ => {}
            },
            Some("response_item") => self.response_item(unit, payload, at),
            _ => {}
        }
    }

    fn finish(self: Box<Self>, consumed: u64, stale: bool) -> Outcome {
        let close = stale || !self.markers;
        self.acc.finish(consumed, close)
    }
}

fn response_text(value: &Value) -> Option<(&str, String)> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    let payload = value.get("payload")?;
    let role = payload.get("role")?.as_str()?;
    Some((role, content_text(payload.get("content")?)))
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
        "Codex session".to_string()
    } else {
        clipped
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_out_recommended_plugins_system_injection() {
        let text = "<recommended_plugins>\nHere is a list of plugins...\n</recommended_plugins>";
        assert_eq!(clean_codex_user_text(text), None);

        let agents_md = "# AGENTS.md instructions for /repo\n<INSTRUCTIONS>...</INSTRUCTIONS>";
        assert_eq!(clean_codex_user_text(agents_md), None);

        let env_ctx = "<environment_context>\n<cwd>/path</cwd>\n</environment_context>";
        assert_eq!(clean_codex_user_text(env_ctx), None);

        let page =
            r#"<external_codex_apps_open_page>{"page_id":null}</external_codex_apps_open_page>"#;
        assert_eq!(clean_codex_user_text(page), None);
        let goal =
            "<codex_internal_context source=\"goal\">\nkeep going\n</codex_internal_context>";
        assert_eq!(clean_codex_user_text(goal), None);
        let attached = "# Files mentioned by the user:\n\n## shot.png: /tmp/shot.png\nImage attachment: true\n\n## My request for Codex:\n看下这个布局";
        assert_eq!(
            clean_codex_user_text(attached),
            Some("看下这个布局".to_string())
        );
        assert_eq!(
            clean_codex_user_text("<div> is a tag I want explained"),
            Some("<div> is a tag I want explained".to_string())
        );
    }

    #[test]
    fn extracts_real_user_prompt_and_strips_jockey_context() {
        let wrapped = "System:\n[Jockey context]\nrole: Dev\n\nUser:\nhello world";
        assert_eq!(
            clean_codex_user_text(wrapped),
            Some("hello world".to_string())
        );

        let direct = "what is the status?";
        assert_eq!(
            clean_codex_user_text(direct),
            Some("what is the status?".to_string())
        );
    }

    #[test]
    fn ignores_recommended_plugins_as_session_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rollout.jsonl");
        let cwd = dir.path().to_str().unwrap();

        std::fs::write(
            &path,
            format!(
                concat!(
                    r#"{{"type":"session_meta","payload":{{"id":"s1","cwd":"{}"}}}}"#,
                    "\n",
                    r#"{{"type":"response_item","payload":{{"role":"user","content":[{{"type":"input_text","text":"<recommended_plugins>\nplugins\n</recommended_plugins>"}}]}}}}"#,
                    "\n",
                    r#"{{"type":"response_item","payload":{{"role":"user","content":[{{"type":"input_text","text":"重构完成。104 个测试通过"}}]}}}}"#,
                    "\n",
                    r#"{{"type":"response_item","payload":{{"role":"assistant","content":[{{"type":"text","text":"很好！"}}]}}}}"#,
                    "\n",
                ),
                cwd
            ),
        )
        .expect("write codex fixture");

        let project_cwd = canonical_path(cwd);
        let session = scan_file(&path, Some(&project_cwd)).expect("session found");
        assert_eq!(session.title, "重构完成。104 个测试通过");
        assert_eq!(session.turn_count, 2);

        let out = crate::importer::tail::read("codex-cli", "Developer", &path, 0, false)
            .unwrap()
            .unwrap();
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[0].role, "user");
        assert_eq!(out.items[0].text, "重构完成。104 个测试通过");
        assert_eq!(out.items[1].role, "Developer");
        assert_eq!(out.items[1].text, "很好！");
        assert!(out.open.is_none(), "a marker-less rollout is stored whole");
    }

    #[test]
    fn marked_turns_group_into_one_message_with_their_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let line = |ts: &str, kind: &str, payload: serde_json::Value| {
            format!(
                "{}\n",
                json!({ "timestamp": ts, "type": kind, "payload": payload })
            )
        };
        let mut text = String::new();
        text += &line(
            "2026-10-21T12:12:00.000Z",
            "event_msg",
            json!({"type":"task_started","turn_id":"T1"}),
        );
        text += &line(
            "2026-10-21T12:12:01.000Z",
            "response_item",
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}),
        );
        text += &line(
            "2026-10-21T12:12:02.000Z",
            "response_item",
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Running ls."}]}),
        );
        text += &line(
            "2026-10-21T12:12:03.000Z",
            "response_item",
            json!({"type":"function_call","call_id":"c1","name":"shell","arguments":"{\"command\":\"ls\"}"}),
        );
        text += &line(
            "2026-10-21T12:12:04.000Z",
            "response_item",
            json!({"type":"function_call_output","call_id":"c1","output":[{"type":"input_text","text":"a.txt"}]}),
        );
        std::fs::write(&path, &text).unwrap();
        let read = || {
            crate::importer::tail::read("codex-cli", "Developer", &path, 0, false)
                .unwrap()
                .unwrap()
        };
        let open = read();
        assert_eq!(
            open.items.len(),
            1,
            "the prompt; the turn waits for task_complete"
        );
        assert_eq!(open.running, Some(true));
        assert!(open.open.is_some());

        text += &line(
            "2026-10-21T12:12:05.000Z",
            "response_item",
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Only a.txt."}]}),
        );
        text += &line(
            "2026-10-21T12:12:06.000Z",
            "event_msg",
            json!({"type":"task_complete","turn_id":"T1"}),
        );
        std::fs::write(&path, &text).unwrap();
        let done = read();
        assert_eq!(done.running, Some(false));
        assert!(done.open.is_none());
        let turn = &done.items[1];
        assert_eq!(turn.id, "t:T1");
        assert_eq!(turn.text, "Running ls.\n\nOnly a.txt.");
        let tc = &turn.msg.as_ref().unwrap()["toolCalls"][0];
        assert_eq!(tc["kind"], "execute");
        assert_eq!(tc["rawInput"]["command"], "ls");
        assert_eq!(tc["rawOutput"], "a.txt");
        assert_eq!(tc["status"], "completed");
    }

    #[test]
    fn subagent_rollouts_are_skipped_and_forks_keep_their_own_id() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let user = r#"{"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"ship it"}]}}"#;
        let write = |name: &str, meta: String| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("{meta}\n{user}\n")).unwrap();
            path
        };
        let fork = write(
            "fork.jsonl",
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"child","session_id":"child","forked_from_id":"root","source":"vscode","cwd":"{cwd}"}}}}"#
            ),
        );
        let guardian = write(
            "guardian.jsonl",
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"g1","session_id":"child","parent_thread_id":"child","source":{{"subagent":{{"other":"guardian"}}}},"cwd":"{cwd}"}}}}"#
            ),
        );
        let root = canonical_path(cwd);
        let found = scan_file(&fork, Some(&root)).expect("fork listed");
        assert_eq!(found.session_id, "child");
        assert_eq!(found.title, "[Fork] ship it");
        assert!(scan_file(&guardian, Some(&root)).is_none());
    }

    #[test]
    fn scan_all_ignores_the_project_filter() {
        let dir = tempfile::tempdir().unwrap();
        for (name, cwd) in [("a", "/work/a"), ("b", "/other/b")] {
            std::fs::write(
                dir.path().join(format!("{name}.jsonl")),
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{name}\",\"cwd\":\"{cwd}\"}}}}\n{}\n",
                    r#"{"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"hi"}]}}"#
                ),
            )
            .unwrap();
        }
        assert_eq!(scan_codex_root(dir.path(), None).len(), 2);
        assert_eq!(scan_codex_root(dir.path(), Some("/work")).len(), 1);
    }
}
