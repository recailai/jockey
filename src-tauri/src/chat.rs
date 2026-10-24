pub(crate) mod prompt_builder;
pub(crate) mod session_runtime;

use crate::chat::session_runtime::load_role_runtime_data;
use crate::db::get_state;
use crate::parser::parse_route_input;
use crate::types::*;
use crate::{acp, clip_text, now_ms};
use serde_json::json;
use std::time::Instant;
use tauri::{AppHandle, State};

pub(crate) fn chat_log(event: &str, payload: serde_json::Value) {
    eprintln!("[jockey.chat] {} {} {}", now_ms(), event, payload);
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

/// Run one turn's agents. Routing (which personas, on which engine) has already been decided
/// by `engine::sessions::route`; the engine records and announces the turn around this call.
pub(crate) async fn run_turn(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AssistantChatInput,
    role_targets: Vec<String>,
    active_role: &str,
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
            role_replies: Vec::new(),
        });
    }

    let assistant = input
        .runtime_kind
        .clone()
        .ok_or_else(|| "assistant not selected".to_string())?;

    let routed = parse_route_input(&text);
    let mut message = routed.prompt.clone();
    if message.is_empty() {
        message = "Please answer based on the attached context.".to_string();
    }

    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), &app_session_id)
        .unwrap_or_else(crate::resolve_chat_cwd);
    let mut role_outputs: Vec<(String, String, bool, Option<String>)> = Vec::new();
    let mut any_acp_error = false;

    for role_name in role_targets {
        let tmp_state = get_state(&state).clone_refs();
        let role_name_clone = role_name.clone();
        // The selected engine belongs to the session's active persona. Any other mentioned
        // persona resolves its own (project pin, bound runtime, persona default) — passing the
        // selection to every role ran e.g. `@agy` on Claude with Gemini's model config.
        let assistant_clone = if role_name.eq_ignore_ascii_case(active_role) {
            assistant.clone()
        } else {
            String::new()
        };
        let app_session_id_clone = app_session_id.clone();
        let db_data = tokio::task::spawn_blocking(move || {
            load_role_runtime_data(
                &tmp_state,
                &app_session_id_clone,
                &role_name_clone,
                &assistant_clone,
            )
        })
        .await
        .map_err(|e| e.to_string())??;

        let runtime = db_data.runtime;
        let auto_approve = db_data.auto_approve;
        let role_mode = db_data.role_mode;
        let role_config = db_data.role_config;
        let mcp_servers = db_data.mcp_servers;

        let prepared = message.clone();

        chat_log(
            "route.acp.start",
            json!({
                "runtime": runtime.clone(),
                "role": role_name.clone(),
                "cwd": cwd.clone(),
                "preparedSize": prepared.len()
            }),
        );
        let _ = crate::db::lifecycle::transition_internal(
            get_state(&state),
            &app_session_id,
            &role_name,
            &runtime,
            "running",
            None,
        );
        let acp_started = Instant::now();
        let llm = acp::execute_runtime(
            &runtime,
            &role_name,
            &prepared,
            &input.attachments,
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

        let final_output = prompt_builder::with_command_suggestion(output);
        let _ = crate::db::lifecycle::transition_internal(
            get_state(&state),
            &app_session_id,
            &role_name,
            &runtime,
            if llm.ok { "ready" } else { "error" },
            if llm.ok {
                None
            } else {
                Some(final_output.as_str())
            },
        );
        if !llm.ok {
            any_acp_error = true;
        }
        role_outputs.push((
            role_name.clone(),
            final_output,
            llm.ok,
            llm.error_code.clone(),
        ));
    }

    let reply = if role_outputs.len() == 1 {
        role_outputs
            .first()
            .map(|(_, output, _, _)| output.clone())
            .unwrap_or_default()
    } else {
        role_outputs
            .iter()
            .map(|(_, output, _, _)| output.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let role_replies = if role_outputs.len() > 1 {
        role_outputs
            .iter()
            .map(|(role, output, ok, error_code)| crate::types::RoleReply {
                role_name: role.clone(),
                reply: output.clone(),
                ok: *ok,
                error_code: error_code.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(AssistantChatResponse {
        ok: !any_acp_error,
        reply,
        runtime_kind: Some(assistant),
        session_id: None,
        role_replies,
    })
}
