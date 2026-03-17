use crate::commands::{apply_chat_command, ensure_team_selected};
use crate::db::context::{
    context_scope_for_role, list_shared_context_internal, set_shared_context_internal,
};
use crate::db::get_state;
use crate::db::role::{load_role, resolve_role_prompt, resolve_role_runtime};
use crate::db::skill::load_skill_by_name;
use crate::types::*;
use crate::{acp, build_unionai_tool_prompt, clip_text, now_ms, resolve_chat_cwd};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::io::Read;
use std::time::Instant;
use tauri::{AppHandle, State};

const RECENT_ROLE_CHATS_KEY: &str = "recentRoleChats";
const RECENT_ROLE_CHATS_LIMIT: usize = 3;
const RECENT_ROLE_CHAT_TEXT_MAX: usize = 1000;

pub(crate) fn parse_route_input(raw: &str) -> ParsedRouteInput {
    let mut out = ParsedRouteInput::default();
    let mut message_tokens = Vec::new();

    for token in raw.split_whitespace() {
        if let Some(skill_ref) = token.strip_prefix('#') {
            let name = strip_trailing_punct(skill_ref).trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                if !out.skill_refs.iter().any(|s| s == name) {
                    out.skill_refs.push(name.to_string());
                }
                continue;
            }
        }
        let Some(rest) = token.strip_prefix('@') else {
            message_tokens.push(token.to_string());
            continue;
        };
        let mention = strip_trailing_punct(rest).trim();
        if mention.is_empty() {
            message_tokens.push(token.to_string());
            continue;
        }

        if let Some(role) = mention.strip_prefix("role:") {
            if let Some(clean_role) = sanitize_role_name(role) {
                if !out
                    .role_names
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&clean_role))
                {
                    out.role_names.push(clean_role);
                }
                continue;
            }
        }
        if let Some(path) = mention.strip_prefix("file:") {
            let clean = path.trim();
            if !clean.is_empty() {
                out.file_refs.push(clean.to_string());
                continue;
            }
        }
        if let Some(path) = mention.strip_prefix("dir:") {
            let clean = path.trim().trim_end_matches('/');
            if !clean.is_empty() {
                out.dir_refs.push(clean.to_string());
                continue;
            }
        }
        if mention == "dir" {
            out.dir_refs.push(".".to_string());
            continue;
        }

        if mention.ends_with('/') {
            let clean = mention.trim_end_matches('/').trim();
            if !clean.is_empty() {
                out.dir_refs.push(clean.to_string());
                continue;
            }
        }
        if mention.ends_with("/**") {
            let clean = mention.trim();
            if !clean.is_empty() {
                out.dir_refs.push(clean.to_string());
                continue;
            }
        }

        if looks_like_path(mention) {
            out.file_refs.push(mention.to_string());
            continue;
        }

        if let Some(clean_role) = sanitize_role_name(mention) {
            if !out
                .role_names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&clean_role))
            {
                out.role_names.push(clean_role);
            }
            continue;
        }

        message_tokens.push(token.to_string());
    }

    out.message = message_tokens.join(" ").trim().to_string();
    out
}

fn strip_trailing_punct(token: &str) -> &str {
    token.trim_end_matches(|c: char| matches!(c, ',' | ';' | '!' | '?'))
}

fn looks_like_path(raw: &str) -> bool {
    raw.contains('/')
        || raw.contains('\\')
        || raw.starts_with("./")
        || raw.starts_with("../")
        || raw.starts_with("~/")
}

fn sanitize_role_name(raw: &str) -> Option<String> {
    let clean = strip_trailing_punct(raw).trim();
    if clean.is_empty() {
        return None;
    }
    if clean
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some(clean.to_string());
    }
    None
}

pub(crate) fn chat_log(event: &str, payload: serde_json::Value) {
    eprintln!("[unionai.chat] {} {} {}", now_ms(), event, payload);
}

fn upsert_context_pair(context_pairs: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some(existing) = context_pairs.iter_mut().find(|(k, _)| k == key) {
        existing.1 = value;
    } else {
        context_pairs.push((key.to_string(), value));
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecentRoleChat {
    role: String,
    user: String,
    assistant: String,
}

fn collapse_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_recent_chat_text(raw: &str) -> String {
    let compact = collapse_whitespace(raw.trim());
    clip_text(&compact, RECENT_ROLE_CHAT_TEXT_MAX)
}

fn load_recent_role_chats(state: &AppState, team_id: &str) -> Vec<RecentRoleChat> {
    let entries = list_shared_context_internal(state, team_id).unwrap_or_default();
    entries
        .into_iter()
        .find(|entry| entry.key == RECENT_ROLE_CHATS_KEY)
        .and_then(|entry| serde_json::from_str::<Vec<RecentRoleChat>>(&entry.value).ok())
        .unwrap_or_default()
}

fn append_recent_role_chat(
    state: &AppState,
    team_id: &str,
    role_name: &str,
    user: &str,
    assistant: &str,
) {
    let mut chats = load_recent_role_chats(state, team_id);
    chats.retain(|c| c.role != role_name);
    chats.push(RecentRoleChat {
        role: role_name.to_string(),
        user: normalize_recent_chat_text(user),
        assistant: normalize_recent_chat_text(assistant),
    });
    if chats.len() > RECENT_ROLE_CHATS_LIMIT {
        let drop_count = chats.len() - RECENT_ROLE_CHATS_LIMIT;
        chats.drain(0..drop_count);
    }
    if let Ok(payload) = serde_json::to_string(&chats) {
        let _ = set_shared_context_internal(state, team_id, RECENT_ROLE_CHATS_KEY, &payload);
    }
}

pub(crate) fn detect_reply_signals(reply: &str) -> Vec<String> {
    let text = reply.to_ascii_lowercase();
    let mut signals = Vec::new();
    if text.contains("memory show")
        || text.contains("memory refresh")
        || text.contains("memory add")
        || text.contains("memory list")
    {
        signals.push("memory".to_string());
    }
    if text.contains("available_commands_update") {
        signals.push("available_commands_update".to_string());
    }
    if text.contains("acp") {
        signals.push("acp".to_string());
    }
    signals
}

pub(crate) fn extract_command_output(reply: &str) -> Option<String> {
    let trimmed = reply.trim();
    if trimmed.starts_with('/') {
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with('`') && trimmed.ends_with('`') {
        let inner = trimmed.trim_matches('`').trim();
        if inner.starts_with('/') {
            return Some(inner.to_string());
        }
    }
    for line in trimmed.lines() {
        let cleaned = line.trim().trim_matches('`').trim();
        if cleaned.starts_with('/') {
            return Some(cleaned.to_string());
        }
    }
    None
}

pub(crate) fn resolve_attach_path(cwd: &str, raw: &str) -> std::path::PathBuf {
    let input = raw.trim();
    let expanded = if input == "~" || input.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if input == "~" {
            home
        } else {
            format!("{home}{}", &input[1..])
        }
    } else {
        input.to_string()
    };
    let candidate = std::path::PathBuf::from(expanded);
    let joined = if candidate.is_absolute() {
        candidate
    } else {
        std::path::PathBuf::from(cwd).join(candidate)
    };
    joined.canonicalize().unwrap_or(joined)
}

fn workspace_root(cwd: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(cwd)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(cwd))
}

pub(crate) fn is_within_workspace(path: &std::path::Path, cwd: &str) -> bool {
    path.starts_with(workspace_root(cwd))
}

pub(crate) fn relative_or_abs(path: &std::path::Path, cwd: &str) -> String {
    let cwd_path = std::path::Path::new(cwd);
    if let Ok(relative) = path.strip_prefix(cwd_path) {
        return relative.to_string_lossy().to_string();
    }
    path.to_string_lossy().to_string()
}

pub(crate) fn should_skip_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "dist"
            | "build"
            | "target"
            | ".idea"
            | ".vscode"
            | ".DS_Store"
    )
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let mut suspicious = 0usize;
    for b in bytes {
        let is_control = (*b < 0x09) || (*b > 0x0D && *b < 0x20);
        if is_control {
            suspicious += 1;
        }
    }
    suspicious * 10 > bytes.len()
}

fn read_text_snippet(
    path: &std::path::Path,
    limit: usize,
) -> Result<(String, usize, bool), String> {
    if limit == 0 {
        return Err("attachment budget exhausted".to_string());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let read_limit = limit.saturating_add(1) as u64;
    let mut reader = file.take(read_limit);
    let mut bytes = Vec::with_capacity(limit.min(4096).saturating_add(1));
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let inspect = bytes.len().min(4096);
    if looks_binary(&bytes[..inspect]) {
        return Err("binary file skipped".to_string());
    }
    let truncated = bytes.len() > limit;
    let take = if truncated { limit } else { bytes.len() };
    let mut text = String::from_utf8_lossy(&bytes[..take]).to_string();
    if truncated {
        text.push_str("\n...[truncated]");
    }
    Ok((text, take, truncated))
}

fn collect_dir_files(
    root: &std::path::Path,
    max_files: usize,
    max_depth: usize,
) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut dirs = Vec::new();
        let mut local_files = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if should_skip_name(&name) {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                if depth < max_depth {
                    dirs.push(path);
                }
                continue;
            }
            if ft.is_file() {
                local_files.push(path);
            }
        }

        dirs.sort();
        local_files.sort();

        for file in local_files {
            files.push(file);
            if files.len() >= max_files {
                return files;
            }
        }

        for child in dirs {
            queue.push_back((child, depth + 1));
        }
    }

    files
}

async fn attach_file_context(
    cwd: String,
    raw_ref: String,
    budget: usize,
) -> Result<(String, String, usize), String> {
    tokio::task::spawn_blocking(move || {
        let path = resolve_attach_path(&cwd, &raw_ref);
        if !is_within_workspace(&path, &cwd) {
            return Err(format!("path is outside workspace: {}", raw_ref));
        }
        if !path.exists() {
            return Err(format!("file not found: {}", raw_ref));
        }
        if path.is_dir() {
            return Err(format!("path is directory (use @dir:): {}", raw_ref));
        }
        let per_file_budget = budget.min(ATTACH_MAX_FILE_BYTES);
        let (snippet, used, _) = read_text_snippet(&path, per_file_budget)?;
        let label = relative_or_abs(&path, &cwd);
        Ok((format!("file:{label}"), snippet, used))
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn attach_dir_context(
    cwd: String,
    raw_ref: String,
    budget: usize,
) -> Result<(String, String, usize), String> {
    if budget == 0 {
        return Err("attachment budget exhausted".to_string());
    }
    tokio::task::spawn_blocking(move || {
        let normalized_ref = if let Some(prefix) = raw_ref.strip_suffix("/**") {
            prefix
        } else {
            &raw_ref
        };
        let path = resolve_attach_path(&cwd, normalized_ref);
        if !is_within_workspace(&path, &cwd) {
            return Err(format!("path is outside workspace: {}", raw_ref));
        }
        if !path.exists() {
            return Err(format!("directory not found: {}", raw_ref));
        }
        if !path.is_dir() {
            return Err(format!("path is file (use @file:): {}", raw_ref));
        }

        let files = collect_dir_files(&path, ATTACH_MAX_DIR_FILES, ATTACH_MAX_DIR_DEPTH);
        if files.is_empty() {
            return Err(format!("directory is empty or unreadable: {}", raw_ref));
        }

        let label = relative_or_abs(&path, &cwd);
        let mut body = format!(
            "Directory: {label}\nFiles (sampled up to {}):",
            ATTACH_MAX_DIR_FILES
        );
        let mut used = body.len();
        for p in &files {
            let rel = relative_or_abs(p, &cwd);
            body.push_str(&format!("\n- {rel}"));
        }
        used += files
            .iter()
            .map(|p| relative_or_abs(p, &cwd).len() + 3)
            .sum::<usize>();
        body.push_str("\n\nContents:");

        for p in files {
            if used >= budget {
                body.push_str("\n...[directory attachment truncated by budget]");
                break;
            }
            let remaining = (budget - used).min(ATTACH_MAX_FILE_BYTES);
            let Ok((snippet, consumed, _)) = read_text_snippet(&p, remaining) else {
                continue;
            };
            if snippet.trim().is_empty() {
                continue;
            }
            let rel = relative_or_abs(&p, &cwd);
            body.push_str(&format!("\n\n### {rel}\n{snippet}"));
            used += consumed + rel.len() + 8;
        }

        Ok((format!("dir:{label}"), body, used.min(budget)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn assistant_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AssistantChatInput,
) -> Result<AssistantChatResponse, String> {
    let started = Instant::now();
    let text = input.input.trim().to_string();
    chat_log(
        "request.start",
        json!({
            "inputSize": text.len(),
            "preview": clip_text(&text, 120),
            "selectedTeamId": input.selected_team_id.clone(),
            "selectedAssistant": input.selected_assistant.clone()
        }),
    );
    if text.is_empty() {
        chat_log(
            "request.empty",
            json!({
                "latencyMs": started.elapsed().as_millis()
            }),
        );
        return Ok(AssistantChatResponse {
            ok: false,
            reply: "empty input".to_string(),
            selected_team_id: input.selected_team_id,
            selected_assistant: input.selected_assistant,
            session_id: None,
            command_result: None,
        });
    }

    if text.starts_with("/app_") {
        let route_started = Instant::now();
        let command_result = apply_chat_command(
            app,
            state,
            text,
            input.selected_team_id.clone(),
            input.selected_assistant.clone(),
        )
        .await?;
        chat_log(
            "route.command",
            json!({
                "latencyMs": route_started.elapsed().as_millis(),
                "totalLatencyMs": started.elapsed().as_millis(),
                "ok": command_result.ok,
                "message": clip_text(&command_result.message, 120)
            }),
        );
        return Ok(AssistantChatResponse {
            ok: command_result.ok,
            reply: command_result.message.clone(),
            selected_team_id: command_result.selected_team_id.clone(),
            selected_assistant: command_result.selected_assistant.clone(),
            session_id: command_result.session_id.clone(),
            command_result: Some(command_result),
        });
    }

    let assistant = input
        .selected_assistant
        .clone()
        .ok_or_else(|| "assistant not selected".to_string())?;

    let routed = parse_route_input(&text);
    let explicit_role_targets = !routed.role_names.is_empty();
    let mut role_targets = if explicit_role_targets {
        routed.role_names.clone()
    } else {
        vec!["UnionAIAssistant".to_string()]
    };
    if role_targets.is_empty() {
        role_targets.push("UnionAIAssistant".to_string());
    }
    let mut message = routed.message;
    if message.is_empty() {
        message = "Please answer based on the attached context.".to_string();
    }

    let team_id = ensure_team_selected(state.clone(), input.selected_team_id.clone())?;
    let tool_prompt = build_unionai_tool_prompt();
    let cwd = resolve_chat_cwd(get_state(&state), Some(&team_id));
    let mut attachment_pairs: Vec<(String, String)> = Vec::new();
    let mut attach_budget = ATTACH_MAX_TOTAL_BYTES;
    let mut attach_notes = Vec::new();
    for file_ref in routed.file_refs {
        if attach_budget == 0 {
            attach_notes.push("attachment budget reached; some files skipped".to_string());
            break;
        }
        match attach_file_context(cwd.clone(), file_ref.clone(), attach_budget).await {
            Ok((key, value, used)) => {
                attachment_pairs.push((key, value));
                attach_budget = attach_budget.saturating_sub(used);
            }
            Err(e) => {
                attach_notes.push(format!("{} ({})", e, file_ref));
            }
        }
    }
    for dir_ref in routed.dir_refs {
        if attach_budget == 0 {
            attach_notes.push("attachment budget reached; some directories skipped".to_string());
            break;
        }
        match attach_dir_context(cwd.clone(), dir_ref.clone(), attach_budget).await {
            Ok((key, value, used)) => {
                attachment_pairs.push((key, value));
                attach_budget = attach_budget.saturating_sub(used);
            }
            Err(e) => {
                attach_notes.push(format!("{} ({})", e, dir_ref));
            }
        }
    }
    let mut skill_pairs: Vec<(String, String)> = Vec::new();
    for skill_name in &routed.skill_refs {
        if let Some(skill) = load_skill_by_name(get_state(&state), skill_name) {
            if !skill.content.is_empty() {
                skill_pairs.push((format!("skill:{}", skill.name), skill.content));
            }
        }
    }
    let mut role_outputs: Vec<(String, String)> = Vec::new();

    for role_name in role_targets {
        let is_union_assistant = role_name == "UnionAIAssistant";
        let mut runtime = assistant.clone();
        if !is_union_assistant {
            runtime = resolve_role_runtime(state.clone(), &team_id, &role_name).unwrap_or(runtime);
        }

        let mut context_pairs: Vec<(String, String)> = Vec::new();
        let scope = context_scope_for_role(&role_name);
        let entries = list_shared_context_internal(get_state(&state), &scope).unwrap_or_default();
        for entry in entries {
            context_pairs.push((entry.key, entry.value));
        }
        if !is_union_assistant {
            let role_prompt =
                resolve_role_prompt(state.clone(), &team_id, &role_name).unwrap_or_default();
            if !role_prompt.is_empty() {
                context_pairs.push(("role_prompt".to_string(), role_prompt));
            }
            let recent_chats: Vec<_> = load_recent_role_chats(get_state(&state), &team_id)
                .into_iter()
                .filter(|c| c.role != role_name)
                .collect();
            if !recent_chats.is_empty() {
                if let Ok(payload) = serde_json::to_string(&recent_chats) {
                    upsert_context_pair(&mut context_pairs, "from_last_role_context", payload);
                }
                chat_log(
                    "route.context.share",
                    json!({
                        "role": role_name.clone(),
                        "recentChatCount": recent_chats.len()
                    }),
                );
            }
        }
        context_pairs.extend(attachment_pairs.clone());
        context_pairs.extend(skill_pairs.clone());
        if !attach_notes.is_empty() {
            context_pairs.push(("attachment_notes".to_string(), attach_notes.join("\n")));
        }
        if !context_pairs.iter().any(|(k, _)| k == "cwd") {
            context_pairs.insert(0, ("cwd".to_string(), cwd.clone()));
        }

        let mut parts = Vec::new();
        if is_union_assistant {
            parts.push(format!("Tools:\n{}", tool_prompt));
        }
        if !context_pairs.is_empty() {
            let ctx = context_pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("Context:\n{}", ctx));
        }
        parts.push(format!("User:\n{}", message));
        let prepared = parts.join("\n\n");

        chat_log(
            "route.acp.start",
            json!({
                "runtime": runtime.clone(),
                "role": role_name.clone(),
                "cwd": cwd.clone(),
                "contextCount": context_pairs.len(),
                "preparedSize": prepared.len()
            }),
        );
        let acp_started = Instant::now();
        let role_data = if !is_union_assistant {
            load_role(get_state(&state), &team_id, &role_name).unwrap_or(None)
        } else {
            None
        };
        let auto_approve = role_data.as_ref().map(|r| r.auto_approve).unwrap_or(true);
        let role_mode = role_data.as_ref().and_then(|r| r.mode.clone());
        let role_config: Vec<(String, String)> = role_data
            .as_ref()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.config_options_json).ok())
            .and_then(|v| {
                v.as_object().map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
            })
            .unwrap_or_default();
        let app_session_id_ref = input.app_session_id.as_deref().unwrap_or("");
        let llm = acp::execute_runtime(
            &runtime,
            &role_name,
            &prepared,
            &[],
            &cwd,
            &app,
            auto_approve,
            vec![],
            role_mode,
            role_config,
            if app_session_id_ref.is_empty() { None } else { Some((get_state(&state), app_session_id_ref)) },
        )
        .await;
        let output = llm.output.trim().to_string();
        let llm_delta_count = llm.deltas.len();
        let llm_meta = llm.meta.clone();
        chat_log(
            "route.acp.done",
            json!({
                "runtime": runtime.clone(),
                "role": role_name.clone(),
                "latencyMs": acp_started.elapsed().as_millis(),
                "totalLatencyMs": started.elapsed().as_millis(),
                "outputSize": output.len(),
                "deltaCount": llm_delta_count,
                "meta": llm_meta
            }),
        );
        let signals = detect_reply_signals(&output);
        if !signals.is_empty() {
            chat_log(
                "route.acp.signal",
                json!({
                    "signals": signals,
                    "preview": clip_text(&output, 220)
                }),
            );
        }

        let mut final_output = output;
        if !explicit_role_targets && is_union_assistant {
            if let Some(command_text) = extract_command_output(&final_output) {
                chat_log(
                    "route.acp.command_output.suggested",
                    json!({
                        "command": clip_text(&command_text, 180)
                    }),
                );
                final_output = format!(
                    "{}\n\n[Command suggestion]\n{}\nRun this command manually if you want to apply it.",
                    final_output, command_text
                );
            }
        }
        if !is_union_assistant {
            append_recent_role_chat(
                get_state(&state),
                &team_id,
                &role_name,
                &message,
                &final_output,
            );
        }
        role_outputs.push((role_name.clone(), final_output));
    }

    let reply = if role_outputs.len() == 1 {
        role_outputs
            .first()
            .map(|(_, output)| output.clone())
            .unwrap_or_default()
    } else {
        role_outputs
            .iter()
            .map(|(role, output)| format!("[{}]\n{}", role, output))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    Ok(AssistantChatResponse {
        ok: true,
        reply,
        selected_team_id: Some(team_id),
        selected_assistant: Some(assistant),
        session_id: None,
        command_result: None,
    })
}
