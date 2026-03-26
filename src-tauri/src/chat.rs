mod session_runtime;

use crate::commands::apply_chat_command;
use crate::db::context::{list_shared_context_internal, set_shared_context_internal};
use crate::db::get_state;
use crate::db::session_context::app_session_scope;
use crate::db::skill::load_skills_by_names;
use crate::fs_context::{attach_dir_context, attach_file_context};
use crate::parser::parse_route_input;
use crate::chat::session_runtime::load_role_runtime_data;
use crate::types::*;
use crate::{acp, build_unionai_tool_prompt, clip_text, now_ms, resolve_chat_cwd};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write as FmtWrite;
use std::time::Instant;
use tauri::{AppHandle, State};

const RECENT_ROLE_CHATS_KEY: &str = "recentRoleChats";
const RECENT_ROLE_CHATS_LIMIT: usize = 9;
const RECENT_ROLE_TURNS_PER_ROLE: usize = 3;
const RECENT_ROLE_CHAT_TEXT_MAX: usize = 5000;

pub(crate) fn chat_log(event: &str, payload: serde_json::Value) {
    eprintln!("[unionai.chat] {} {} {}", now_ms(), event, payload);
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecentRoleChat {
    role: String,
    user: String,
    assistant: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    cwd: String,
}

fn collapse_whitespace(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_space = true;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn normalize_recent_chat_text(raw: &str) -> String {
    let compact = collapse_whitespace(raw.trim());
    clip_text(&compact, RECENT_ROLE_CHAT_TEXT_MAX)
}

fn load_recent_role_chats(state: &AppState, app_session_id: &str) -> Vec<RecentRoleChat> {
    let scope = app_session_scope(app_session_id);
    let entries = list_shared_context_internal(state, &scope).unwrap_or_default();
    entries
        .into_iter()
        .find(|entry| entry.key == RECENT_ROLE_CHATS_KEY)
        .and_then(|entry| serde_json::from_str::<Vec<RecentRoleChat>>(&entry.value).ok())
        .unwrap_or_default()
}

fn append_recent_role_chat(
    state: &AppState,
    role_name: &str,
    user: &str,
    assistant: &str,
    cwd: &str,
    app_session_id: &str,
) {
    let mut chats = load_recent_role_chats(state, app_session_id);
    // Trim this role's history to keep only the most recent (TURNS_PER_ROLE - 1)
    // entries, then append the new one — so at most TURNS_PER_ROLE per role.
    let mut role_count = chats.iter().filter(|c| c.role == role_name).count();
    while role_count >= RECENT_ROLE_TURNS_PER_ROLE {
        if let Some(pos) = chats.iter().position(|c| c.role == role_name) {
            chats.remove(pos);
            role_count -= 1;
        } else {
            break;
        }
    }
    chats.push(RecentRoleChat {
        role: role_name.to_string(),
        user: normalize_recent_chat_text(user),
        assistant: normalize_recent_chat_text(assistant),
        cwd: cwd.to_string(),
    });
    // Global cap across all roles.
    if chats.len() > RECENT_ROLE_CHATS_LIMIT {
        let drop_count = chats.len() - RECENT_ROLE_CHATS_LIMIT;
        chats.drain(0..drop_count);
    }
    if let Ok(payload) = serde_json::to_string(&chats) {
        let scope = app_session_scope(app_session_id);
        let _ = set_shared_context_internal(state, &scope, RECENT_ROLE_CHATS_KEY, &payload);
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

#[tauri::command]
pub(crate) async fn assistant_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AssistantChatInput,
) -> Result<AssistantChatResponse, String> {
    let started = Instant::now();
    let text = input.input.trim().to_string();
    let app_session_id = input
        .app_session_id
        .clone()
        .filter(|sid| !sid.trim().is_empty())
        .ok_or_else(|| "app session id required".to_string())?;
    chat_log(
        "request.start",
        json!({
            "inputSize": text.len(),
            "preview": clip_text(&text, 120),
            "runtimeKind": input.runtime_kind.clone()
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
            runtime_kind: input.runtime_kind,
            session_id: None,
            command_result: None,
        });
    }

    if text.starts_with("/app_") {
        let route_started = Instant::now();
        let command_result =
            apply_chat_command(app, state, text, input.runtime_kind.clone(), Some(app_session_id.clone())).await?;
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
            runtime_kind: command_result.runtime_kind.clone(),
            session_id: command_result.session_id.clone(),
            command_result: Some(command_result),
        });
    }

    let assistant = input
        .runtime_kind
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

    let tool_prompt = build_unionai_tool_prompt();
    let cwd = input.app_session_id.as_deref()
        .and_then(|sid| crate::db::app_session::get_app_session_cwd(crate::db::get_state(&state), sid))
        .unwrap_or_else(resolve_chat_cwd);
    let mut attachment_pairs: Vec<(String, String)> = Vec::new();
    let mut attach_budget = ATTACH_MAX_TOTAL_BYTES;
    let mut attach_notes = Vec::new();

    {
        let per_file_budget =
            attach_budget / (routed.file_refs.len() + routed.dir_refs.len()).max(1);
        let per_file_budget = per_file_budget.min(ATTACH_MAX_TOTAL_BYTES);
        let file_futs: Vec<_> = routed
            .file_refs
            .iter()
            .map(|r| attach_file_context(cwd.clone(), r.clone(), per_file_budget))
            .collect();
        let dir_futs: Vec<_> = routed
            .dir_refs
            .iter()
            .map(|r| attach_dir_context(cwd.clone(), r.clone(), per_file_budget))
            .collect();
        let (file_results, dir_results) = tokio::join!(
            futures::future::join_all(file_futs),
            futures::future::join_all(dir_futs)
        );
        for result in file_results.into_iter().chain(dir_results) {
            if attach_budget == 0 {
                attach_notes.push("attachment budget reached; some files skipped".to_string());
                break;
            }
            match result {
                Ok((key, value, used)) => {
                    attachment_pairs.push((key, value));
                    attach_budget = attach_budget.saturating_sub(used);
                }
                Err(e) => attach_notes.push(e),
            }
        }
    }
    // Extract Send-able handles once so spawn_blocking closures can own them.
    let db_pool = get_state(&state).db.clone();
    let shared_ctx = get_state(&state).shared_context.clone();

    let skill_refs = routed.skill_refs.clone();
    let skill_pool = db_pool.clone();
    let skill_ctx = shared_ctx.clone();
    let skill_pairs: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
        let tmp_state = AppState {
            db: skill_pool,
            shared_context: skill_ctx,
        };
        load_skills_by_names(&tmp_state, &skill_refs)
    })
    .await
    .unwrap_or_default()
    .into_iter()
    .filter(|s| !s.content.is_empty())
    .map(|s| (format!("skill:{}", s.name), s.content))
    .collect();
    let pre_pool = db_pool.clone();
    let pre_ctx = shared_ctx.clone();
    let pre_app_session_id = app_session_id.clone();
    let all_recent_chats: Vec<RecentRoleChat> = tokio::task::spawn_blocking(move || {
        let tmp = AppState { db: pre_pool, shared_context: pre_ctx };
        load_recent_role_chats(&tmp, &pre_app_session_id)
    })
    .await
    .unwrap_or_default();

    let mut role_outputs: Vec<(String, String)> = Vec::new();

    for role_name in role_targets {
        let is_union_assistant = role_name == "UnionAIAssistant";
        let pool_clone = db_pool.clone();
        let ctx_clone = shared_ctx.clone();
        let role_name_clone = role_name.clone();
        let assistant_clone = assistant.clone();
        let app_session_id_clone = app_session_id.clone();
        let recent_chats_snapshot = all_recent_chats.clone();
        let db_data = tokio::task::spawn_blocking(move || {
            let tmp_state = AppState {
                db: pool_clone,
                shared_context: ctx_clone,
            };
            load_role_runtime_data(
                &tmp_state,
                &app_session_id_clone,
                &role_name_clone,
                &assistant_clone,
                recent_chats_snapshot,
            )
        })
        .await
        .map_err(|e| e.to_string())?;

        let runtime = db_data.runtime;
        let mut context_pairs = db_data.context_pairs;
        let auto_approve = db_data.auto_approve;
        let role_mode = db_data.role_mode;
        let role_config = db_data.role_config;
        let role_system_prompt = db_data.role_system_prompt;
        let mcp_servers = db_data.mcp_servers;

        if let Some((count, inherited_cwd)) = db_data.context_log {
            chat_log(
                "route.context.share",
                json!({
                    "role": role_name.clone(),
                    "recentChatCount": count,
                    "inheritedCwd": inherited_cwd
                }),
            );
        }

        context_pairs.extend(attachment_pairs.iter().cloned());
        context_pairs.extend(skill_pairs.iter().cloned());
        if !attach_notes.is_empty() {
            context_pairs.push(("attachment_notes".to_string(), attach_notes.join("\n")));
        }
        if !context_pairs.iter().any(|(k, _)| k == "cwd") {
            context_pairs.insert(0, ("cwd".to_string(), cwd.clone()));
        }

        let ctx_bytes: usize = context_pairs
            .iter()
            .map(|(k, v)| k.len() + v.len() + 4)
            .sum();
        let estimated = if is_union_assistant {
            tool_prompt.len() + 10
        } else {
            0
        } + ctx_bytes
            + message.len()
            + 64;
        let mut prepared = String::with_capacity(estimated);
        if is_union_assistant {
            prepared.push_str("Tools:\n");
            prepared.push_str(tool_prompt);
            if let Some(ref sp) = role_system_prompt {
                prepared.push_str("\n\nSystem:\n");
                prepared.push_str(sp);
            }
        }
        let is_slash_cmd = message.starts_with('/');
        if !is_slash_cmd {
            if !context_pairs.is_empty() {
                if !prepared.is_empty() {
                    prepared.push_str("\n\n");
                }
                prepared.push_str("Context:\n");
                for (i, (k, v)) in context_pairs.iter().enumerate() {
                    if i > 0 {
                        prepared.push('\n');
                    }
                    let _ = write!(prepared, "{}: {}", k, v);
                }
            }
            if !prepared.is_empty() {
                prepared.push_str("\n\n");
            }
            prepared.push_str("User:\n");
        }
        prepared.push_str(&message);

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
        let llm = acp::execute_runtime(
            &runtime,
            &role_name,
            &prepared,
            &[],
            &cwd,
            &app,
            auto_approve,
            mcp_servers,
            role_mode,
            role_config,
            Some((get_state(&state), &app_session_id)),
            &app_session_id,
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
                &role_name,
                &message,
                &final_output,
                &cwd,
                &app_session_id,
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
        runtime_kind: Some(assistant),
        session_id: None,
        command_result: None,
    })
}
