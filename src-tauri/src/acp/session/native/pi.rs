use super::super::super::adapter::NativeProtocol;
use super::super::super::runtime_state::{
    has_discovered_models, remember_runtime_available_commands, remember_runtime_models,
};
use super::super::super::worker::AcpEvent;
use super::{
    compose_prompt, extract_pi_session_id, find_option, first_text, native_model_options,
    NativeCatalog, NativeEventSink, NativeProcess, NativeRunRequest, CONTROL_TIMEOUT, TURN_TIMEOUT,
};
use serde_json::{json, Value};

pub(super) async fn run(
    process: &mut NativeProcess,
    request: &NativeRunRequest<'_>,
    sink: &mut NativeEventSink<'_>,
    load_commands: bool,
) -> Result<(String, String, u32), String> {
    let runtime_key = request.runtime_key;
    let role_name = request.role_name;
    let prompt = request.prompt;
    let context = request.context;
    let attachments = request.attachments;
    let auto_approve = request.auto_approve;
    let options = request.role_config_options;
    let app_session_id = request.app_session_id;
    if !request.mcp_servers.is_empty() {
        sink.emit(AcpEvent::StatusUpdate {
            text: format!(
                "{} MCP server(s) bound to this role are ignored: native Pi has no MCP mapping.",
                request.mcp_servers.len()
            ),
        });
        crate::acp::adapter::acp_log(
            "native.pi.mcp_unmapped",
            json!({ "runtime": runtime_key, "count": request.mcp_servers.len() }),
        );
    }
    let (state, state_messages) = process
        .request(
            NativeProtocol::PiRpc,
            "get_state",
            json!({}),
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await?;
    for message in state_messages {
        process_message(&message, sink, &mut String::new())?;
    }
    if !has_discovered_models(runtime_key) {
        let (models, _) = process
            .request(
                NativeProtocol::PiRpc,
                "get_available_models",
                json!({}),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await?;
        if let Some(models) = extract_models(&models) {
            remember_runtime_models(runtime_key, models);
        }
    }
    if let Some(model) = find_option(options, &["model", "model_id"]) {
        if let Some((provider, model_id)) = model.split_once('/') {
            process
                .request(
                    NativeProtocol::PiRpc,
                    "set_model",
                    json!({ "provider": provider, "modelId": model_id }),
                    CONTROL_TIMEOUT,
                    auto_approve,
                )
                .await?;
        }
    }
    if let Some(thinking) = find_option(options, &["thinking", "thinking_level", "effort"]) {
        process
            .request(
                NativeProtocol::PiRpc,
                "set_thinking_level",
                json!({ "level": thinking }),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await?;
    }
    if load_commands {
        if let Ok((commands, _)) = process
            .request(
                NativeProtocol::PiRpc,
                "get_commands",
                json!({}),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await
        {
            if let Some(commands) = commands.get("commands").and_then(Value::as_array) {
                remember_runtime_available_commands(
                    app_session_id,
                    runtime_key,
                    role_name,
                    commands.clone(),
                );
                sink.emit(AcpEvent::AvailableCommands {
                    commands: commands.clone(),
                });
            }
        }
    }
    let input = compose_prompt(prompt, context);
    let images = attachments
        .iter()
        .map(|attachment| {
            json!({
                "type": "image",
                "data": attachment.data.clone(),
                "mimeType": attachment.mime_type.clone(),
            })
        })
        .collect::<Vec<_>>();
    let (ack, prompt_messages) = process
        .request(
            NativeProtocol::PiRpc,
            "prompt",
            json!({ "message": input, "images": images }),
            TURN_TIMEOUT,
            auto_approve,
        )
        .await?;
    let mut output = String::new();
    let mut completed = false;
    for message in prompt_messages {
        completed |= process_message(&message, sink, &mut output)?;
    }
    if ack.get("agentInvoked").and_then(Value::as_bool) == Some(false) {
        completed = true;
    }
    while !completed {
        let message = process
            .next_agent_message(TURN_TIMEOUT, auto_approve)
            .await?;
        completed = process_message(&message, sink, &mut output)?;
    }
    let session_id = if let Some(session_id) = extract_pi_session_id(&state) {
        Some(session_id)
    } else {
        process
            .request(
                NativeProtocol::PiRpc,
                "get_state",
                json!({}),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await
            .ok()
            .and_then(|(value, _)| extract_pi_session_id(&value))
    };
    // Surface turn usage/token stats through diagnostics; the RPC method
    // exists on pi 0.84+ (verified against the installed CLI).
    if let Some((stats, _)) = process
        .request(
            NativeProtocol::PiRpc,
            "get_session_stats",
            json!({}),
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await
        .ok()
    {
        crate::acp::adapter::acp_log(
            "native.pi.usage",
            json!({
                "runtime": runtime_key,
                "role": role_name,
                "stats": stats,
            }),
        );
    }
    Ok((output, session_id.unwrap_or_default(), sink.sequence()))
}

pub(super) fn process_message(
    message: &Value,
    sink: &mut NativeEventSink<'_>,
    output: &mut String,
) -> Result<bool, String> {
    let message_type = message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match message_type {
        "message_update" => {
            let event = message
                .get("assistantMessageEvent")
                .or_else(|| message.get("assistant_message_event"))
                .unwrap_or(&Value::Null);
            match event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "text_delta" => {
                    if let Some(delta) = first_text(event, &["delta", "text"]) {
                        sink.push_text(&delta, output);
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = first_text(event, &["delta", "thinking", "text"]) {
                        sink.emit(AcpEvent::ThoughtDelta { text });
                    }
                }
                _ => {}
            }
        }
        "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
            let id = first_text(message, &["toolCallId", "tool_call_id"])
                .unwrap_or_else(|| "pi-tool".to_string());
            let title = first_text(message, &["toolName", "tool_name"])
                .unwrap_or_else(|| "Pi tool".to_string());
            let status = if message_type.ends_with("end") {
                "completed"
            } else {
                "running"
            };
            sink.emit(AcpEvent::ToolCallUpdate {
                tool_call_id: id,
                tool_kind: Some("tool".to_string()),
                status: Some(status.to_string()),
                title: Some(title),
                content: None,
                locations: None,
                raw_input: message.get("args").cloned(),
                raw_output: message
                    .get("result")
                    .or_else(|| message.get("partialResult"))
                    .cloned(),
                terminal_meta: None,
            });
        }
        "agent_end" | "agent_settled" => return Ok(true),
        "process_exit" => {
            return Err(first_text(message, &["error", "message"])
                .unwrap_or_else(|| "Pi RPC process exited".to_string()))
        }
        _ => {}
    }
    Ok(false)
}

pub(super) async fn refresh_catalog(
    process: &mut NativeProcess,
    runtime_key: &'static str,
    auto_approve: bool,
) -> NativeCatalog {
    let state = process
        .request(
            NativeProtocol::PiRpc,
            "get_state",
            json!({}),
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await
        .ok()
        .map(|(value, _)| value);
    let Some(models) = process
        .request(
            NativeProtocol::PiRpc,
            "get_available_models",
            json!({}),
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await
        .ok()
        .and_then(|(value, _)| extract_models(&value))
    else {
        return NativeCatalog {
            options: vec![],
            modes: vec![],
            session_id: String::new(),
        };
    };
    remember_runtime_models(runtime_key, models.clone());
    NativeCatalog {
        options: native_model_options(
            &models,
            "thinking_level",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        ),
        modes: vec![],
        session_id: state
            .as_ref()
            .and_then(extract_pi_session_id)
            .unwrap_or_default(),
    }
}

fn extract_models(value: &Value) -> Option<Vec<String>> {
    value
        .get("models")
        .or_else(|| value.as_array().map(|_| value))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row.get("id").and_then(Value::as_str)?;
                    let provider = row.get("provider").and_then(Value::as_str);
                    Some(
                        provider
                            .map(|provider| format!("{provider}/{id}"))
                            .unwrap_or_else(|| id.to_string()),
                    )
                })
                .collect()
        })
}
