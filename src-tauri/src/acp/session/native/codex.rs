use super::super::super::adapter::{acp_log, NativeProtocol};
use super::super::super::runtime_state::{has_discovered_models, remember_runtime_models};
use super::super::super::worker::AcpEvent;
use super::{
    compose_prompt, find_option, first_text, native_model_options, NativeCatalog, NativeEventSink,
    NativeProcess, NativeRunRequest, CONTROL_TIMEOUT, TURN_TIMEOUT,
};
use serde_json::{json, Value};

pub(super) async fn run(
    process: &mut NativeProcess,
    request: &NativeRunRequest<'_>,
    sink: &mut NativeEventSink<'_>,
    initialize: bool,
) -> Result<(String, String, u32), String> {
    let runtime_key = request.runtime_key;
    let prompt = request.prompt;
    let context = request.context;
    let attachments = request.attachments;
    let cwd = request.cwd;
    let auto_approve = request.auto_approve;
    let role_mode = request.role_mode;
    let options = request.role_config_options;
    let resume_session_id = request.resume_session_id;
    let mcp_servers = request.mcp_servers;
    if initialize {
        let (_, initialize_messages) = process
            .request(
                NativeProtocol::CodexAppServer,
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "jockey",
                        "title": "Jockey",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "mcpServerOpenaiFormElicitation": true
                    }
                }),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await?;
        for message in initialize_messages {
            process_message(&message, sink, &mut String::new())?;
        }
        process
            .send(json!({ "method": "initialized", "params": {} }))
            .await?;
    }

    if !has_discovered_models(runtime_key) {
        let models = process
            .request(
                NativeProtocol::CodexAppServer,
                "model/list",
                json!({}),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await
            .ok()
            .and_then(|(value, _)| extract_models(&value));
        if let Some(models) = models {
            remember_runtime_models(runtime_key, models);
        }
    }

    let model = find_option(options, &["model", "model_id"]);
    let effort = find_option(options, &["effort", "reasoning_effort", "reasoningEffort"]);
    let policy = if role_mode.map(|mode| mode.eq_ignore_ascii_case("plan")) == Some(true) {
        Some("on-request")
    } else if auto_approve {
        Some("never")
    } else {
        None
    };
    let mut thread_params = json!({ "cwd": cwd });
    if let Some(model) = model.as_deref() {
        thread_params["model"] = json!(model);
    }
    if let Some(policy) = policy {
        thread_params["approvalPolicy"] = json!(policy);
    }
    if !mcp_servers.is_empty() {
        // The Codex app-server protocol (verified against codex 0.150.1 v2
        // schema) has no per-thread MCP channel: servers are config.toml-level
        // only. Surface a visible warning instead of silently dropping the
        // role's MCP bindings.
        sink.emit(AcpEvent::StatusUpdate {
            text: format!(
                "{} MCP server(s) bound to this role are ignored: native Codex only supports MCP via ~/.codex/config.toml.",
                mcp_servers.len()
            ),
        });
        acp_log(
            "native.codex.mcp_unmapped",
            json!({ "runtime": runtime_key, "count": mcp_servers.len() }),
        );
    }

    let thread = if let Some(resume) = resume_session_id.filter(|value| !value.trim().is_empty()) {
        let resumed = process
            .request(
                NativeProtocol::CodexAppServer,
                "thread/resume",
                json!({ "threadId": resume, "cwd": cwd }),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await;
        match resumed {
            Ok(value) if extract_thread_id(&value.0).is_some() => Ok(value),
            _ => {
                process
                    .request(
                        NativeProtocol::CodexAppServer,
                        "thread/start",
                        thread_params,
                        CONTROL_TIMEOUT,
                        auto_approve,
                    )
                    .await
            }
        }
    } else {
        process
            .request(
                NativeProtocol::CodexAppServer,
                "thread/start",
                thread_params,
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await
    }?;
    let thread_id = extract_thread_id(&thread.0)
        .ok_or_else(|| "Codex app-server did not return a thread id".to_string())?;
    let mut input = compose_prompt(prompt, context);
    if !attachments.is_empty() {
        input.push_str(&format!(
            "\n\n[{} image attachment(s) require a native Codex image-input mapping and were omitted]",
            attachments.len()
        ));
    }
    let mut turn_params = json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": input }],
        "cwd": cwd,
    });
    if let Some(model) = model.as_deref() {
        turn_params["model"] = json!(model);
    }
    if let Some(effort) = effort.as_deref() {
        turn_params["effort"] = json!(effort);
    }
    if let Some(policy) = policy {
        turn_params["approvalPolicy"] = json!(policy);
    }
    let (_, messages) = process
        .request(
            NativeProtocol::CodexAppServer,
            "turn/start",
            turn_params,
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await?;
    let mut output = String::new();
    let mut completed = false;
    for message in messages {
        completed |= process_message(&message, sink, &mut output)?;
    }
    while !completed {
        let message = process
            .next_agent_message(TURN_TIMEOUT, auto_approve)
            .await?;
        completed = process_message(&message, sink, &mut output)?;
    }
    Ok((output, thread_id, sink.sequence()))
}

pub(super) fn process_message(
    message: &Value,
    sink: &mut NativeEventSink<'_>,
    output: &mut String,
) -> Result<bool, String> {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "item/agentMessage/delta" => {
            if let Some(delta) = first_text(params, &["delta", "text"]) {
                sink.push_text(&delta, output);
            }
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            if let Some(text) = first_text(params, &["delta", "text"]) {
                sink.emit(AcpEvent::ThoughtDelta { text });
            }
        }
        "item/started" | "item/completed" => {
            if let Some(item) = params.get("item") {
                let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if !kind.eq_ignore_ascii_case("agentMessage")
                    && !kind.eq_ignore_ascii_case("reasoning")
                {
                    sink.emit(AcpEvent::ToolCallUpdate {
                        tool_call_id: item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("codex-item")
                            .to_string(),
                        tool_kind: Some(kind.to_string()),
                        status: Some(
                            if method.ends_with("completed") {
                                "completed"
                            } else {
                                "running"
                            }
                            .to_string(),
                        ),
                        title: first_text(item, &["type", "command", "name"]),
                        content: None,
                        locations: None,
                        raw_input: Some(item.clone()),
                        raw_output: None,
                        terminal_meta: None,
                    });
                }
            }
        }
        "turn/completed" | "turn/failed" | "turn/canceled" | "turn/cancelled" => {
            let status = params
                .get("turn")
                .and_then(|turn| turn.get("status"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if method.contains("failed")
                || method.contains("canceled")
                || method.contains("cancelled")
                || matches!(
                    status.as_str(),
                    "failed" | "error" | "cancelled" | "canceled" | "interrupted"
                )
            {
                return Err(first_text(params, &["error", "message"])
                    .unwrap_or_else(|| format!("Codex turn ended with status {status}")));
            }
            return Ok(true);
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
    let initialized = process
        .request(
            NativeProtocol::CodexAppServer,
            "initialize",
            json!({
                "clientInfo": { "name": "jockey", "title": "Jockey", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {
                    "experimentalApi": true,
                    "mcpServerOpenaiFormElicitation": true
                }
            }),
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await;
    if initialized.is_err() {
        return NativeCatalog {
            options: vec![],
            modes: vec![],
            session_id: String::new(),
        };
    }
    let _ = process
        .send(json!({ "method": "initialized", "params": {} }))
        .await;
    let Some(models) = process
        .request(
            NativeProtocol::CodexAppServer,
            "model/list",
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
            "reasoning_effort",
            &["minimal", "low", "medium", "high", "xhigh"],
        ),
        modes: vec![],
        session_id: String::new(),
    }
}

fn extract_models(value: &Value) -> Option<Vec<String>> {
    let rows = value
        .get("data")
        .or_else(|| value.get("models"))
        .or_else(|| value.as_array().map(|_| value))
        .and_then(Value::as_array)?;
    Some(
        rows.iter()
            .filter_map(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect(),
    )
}

fn extract_thread_id(value: &Value) -> Option<String> {
    value
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("threadId").and_then(Value::as_str))
        .map(ToString::to_string)
}
