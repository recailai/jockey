use super::super::super::adapter::{acp_log, NativeProtocol};
use super::super::super::runtime_state::{
    has_discovered_models, list_discovered_config_options, remember_runtime_models,
};
use super::super::super::worker::AcpEvent;
use super::super::option_spec::{OptionSpec, OptionValue, Wire};
use super::approval;
use super::{
    compose_prompt, find_option, first_text, resolve_wired_values, NativeCatalog, NativeEventSink,
    NativeProcess, NativeRunRequest, CONTROL_TIMEOUT, TURN_TIMEOUT,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

const ALLOWED_IMAGE_MIME: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
];

/// Codex app-server's `turn/start` input only accepts images as on-disk paths
/// (`{"type":"localImage","path":...}`, per the app-server schema), unlike ACP's inline
/// base64 blocks — so each attachment is decoded and written to a temp file first.
fn materialize_codex_images(attachments: &[crate::types::ImageAttachment]) -> Vec<Value> {
    let mut items = Vec::new();
    for attachment in attachments {
        let Some(ext) = ALLOWED_IMAGE_MIME
            .iter()
            .find(|(mime, _)| *mime == attachment.mime_type)
            .map(|(_, ext)| *ext)
        else {
            continue;
        };
        let valid_base64 = !attachment.data.is_empty()
            && attachment
                .data
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
        if !valid_base64 {
            continue;
        }
        let Ok(bytes) = STANDARD.decode(&attachment.data) else {
            continue;
        };
        let dir = std::env::temp_dir().join("jockey-codex-images");
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
        if std::fs::write(&path, &bytes).is_err() {
            continue;
        }
        items.push(json!({ "type": "localImage", "path": path.to_string_lossy() }));
    }
    items
}

pub(super) async fn run(
    process: &mut NativeProcess,
    request: &NativeRunRequest<'_>,
    sink: &mut NativeEventSink<'_>,
    initialize: bool,
    native_key: &str,
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

        let skills_res = process
            .request(
                NativeProtocol::CodexAppServer,
                "skills/list",
                json!({ "cwds": [cwd] }),
                CONTROL_TIMEOUT,
                auto_approve,
            )
            .await
            .ok();
        if let Some((value, _)) = skills_res {
            let mut cmds = Vec::new();
            if let Some(data) = value.get("data").and_then(Value::as_array) {
                for item in data {
                    if let Some(skills) = item.get("skills").and_then(Value::as_array) {
                        for skill in skills {
                            if let Some(name) = skill.get("name").and_then(Value::as_str) {
                                let desc = skill
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                cmds.push(json!({
                                    "name": name,
                                    "description": desc,
                                    "kind": "skill"
                                }));
                            }
                        }
                    }
                }
            }
            if !cmds.is_empty() {
                super::super::super::runtime_state::remember_runtime_available_commands(
                    request.app_session_id,
                    runtime_key,
                    request.role_name,
                    cmds.clone(),
                );
                sink.emit(AcpEvent::AvailableCommands { commands: cmds });
            }
        }
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
            remember_runtime_models(runtime_key, models.iter().map(|m| m.id.clone()).collect());
        }
    }

    // Follow what the runtime declared for each parameter. The `find_option` fallbacks below
    // cover the one case the declaration cannot: discovery has not run yet in this process,
    // so the catalog is empty and a turn must still go out with the user's settings.
    let wired = resolve_wired_values(&list_discovered_config_options(runtime_key), options);
    let model = wired
        .turn_param("model")
        .map(ToString::to_string)
        .or_else(|| find_option(options, &["model", "model_id"]));
    let effort = wired
        .turn_param("effort")
        .map(ToString::to_string)
        .or_else(|| find_option(options, &["effort", "reasoning_effort", "reasoningEffort"]));
    // "Fast mode" is a service-tier override. `TurnStartParams.serviceTier` is documented in
    // Codex's own generated schema. The tier id is read per model, since two models may
    // advertise different tiers and the declaration can only carry one.
    let fast_enabled = find_option(options, &["fast", "fast_mode", "fastMode"])
        .map(|value| matches!(value.trim(), "true" | "1" | "on"))
        .unwrap_or(false);
    let service_tier = if fast_enabled {
        model
            .as_deref()
            .and_then(|selected| fast_tier_for_model(runtime_key, selected))
            .or_else(|| wired.turn_param("serviceTier").map(ToString::to_string))
    } else {
        None
    };
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
    let input = compose_prompt(prompt, context);
    let mut input_items = vec![json!({ "type": "text", "text": input })];
    input_items.extend(materialize_codex_images(attachments));
    let mut turn_params = json!({
        "threadId": thread_id,
        "input": input_items,
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
    if let Some(tier) = service_tier.as_deref() {
        turn_params["serviceTier"] = json!(tier);
    }
    let (turn_start, messages) = process
        .request(
            NativeProtocol::CodexAppServer,
            "turn/start",
            turn_params,
            CONTROL_TIMEOUT,
            auto_approve,
        )
        .await?;
    super::set_active_native_turn(
        native_key,
        NativeProtocol::CodexAppServer,
        Some(thread_id.clone()),
        extract_turn_id(&turn_start),
    );
    let mut output = String::new();
    let mut completed = false;
    for message in messages {
        completed |= process_message(&message, sink, &mut output)?;
    }
    let mut approval_seq = 0u32;
    while !completed {
        let message = process.next_frame(TURN_TIMEOUT).await?;
        if let Some(id) = NativeProcess::server_request_id(&message) {
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let result = if method == approval::USER_INPUT_METHOD {
                // Always ask: a question is not a permission, so auto-approve has no
                // sensible answer to substitute.
                approval_seq += 1;
                let key = format!("{runtime_key}:{thread_id}:q{approval_seq}");
                approval::resolve_user_input(sink, &params, key).await
            } else if auto_approve || !approval::is_approval_method(&method) {
                // No interactive surface needed: keep the previous auto-answer behaviour so
                // unattended runs and non-approval requests do not stall the turn.
                if approval::is_approval_method(&method) {
                    json!({ "decision": approval::auto_decision(&method) })
                } else {
                    json!({})
                }
            } else {
                approval_seq += 1;
                let key = format!("{runtime_key}:{thread_id}:{approval_seq}");
                approval::resolve_approval(sink, &method, &params, key).await
            };
            process.send(json!({ "id": id, "result": result })).await?;
            continue;
        }
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
    if method.is_empty() {
        if let Some(control_id) = message.get("id").and_then(Value::as_str) {
            if super::resolve_native_control_response(control_id, message) {
                return Ok(false);
            }
        }
        if let Some(err_val) = message.get("error") {
            if let Some(msg) = extract_error_message(err_val) {
                return Err(msg);
            }
        }
    }
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
                if kind.eq_ignore_ascii_case("todo_list") || kind.eq_ignore_ascii_case("todoList") {
                    if let Some(entries) = item.get("items").and_then(Value::as_array) {
                        sink.emit(AcpEvent::Plan {
                            entries: entries.clone(),
                        });
                    }
                    return Ok(false);
                }
                if !kind.eq_ignore_ascii_case("agentMessage")
                    && !kind.eq_ignore_ascii_case("reasoning")
                {
                    sink.emit(AcpEvent::ToolCallUpdate {
                        tool_call_id: item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("codex-item")
                            .to_string(),
                        tool_name: first_text(item, &["name", "tool", "toolName"]),
                        parent_id: first_text(item, &["parentId", "parent_id"]),
                        diff: item.get("changes").cloned(),
                        tool_kind: Some(kind.to_string()),
                        status: Some(codex_tool_status(item, method)),
                        title: first_text(item, &["type", "command", "name"]),
                        content: None,
                        locations: None,
                        raw_input: Some(item.clone()),
                        raw_output: item
                            .get("aggregatedOutput")
                            .or_else(|| item.get("output"))
                            .or_else(|| item.get("result"))
                            .or_else(|| item.get("contentItems"))
                            .cloned(),
                        terminal_meta: None,
                    });
                }
            }
        }
        // Long-running commands stream their stdout; forwarding it as a delta avoids
        // resending the whole tool call on every chunk.
        "item/commandExecution/outputDelta"
        | "item/fileChange/outputDelta"
        | "command/exec/outputDelta"
        | "process/outputDelta" => {
            if let (Some(id), Some(delta)) = (
                first_text(params, &["itemId", "item_id", "id"]),
                first_text(params, &["delta", "chunk", "text", "output"]),
            ) {
                sink.emit(AcpEvent::ToolOutputDelta {
                    tool_call_id: id,
                    delta,
                });
            }
        }
        // A patch preview, delivered separately from the item itself.
        "item/fileChange/patchUpdated" => {
            if let Some(id) = first_text(params, &["itemId", "item_id", "id"]) {
                sink.emit(AcpEvent::ToolCallUpdate {
                    tool_call_id: id,
                    tool_name: None,
                    tool_kind: Some("file_change".to_string()),
                    status: None,
                    title: None,
                    content: None,
                    locations: None,
                    raw_input: None,
                    raw_output: None,
                    terminal_meta: None,
                    parent_id: None,
                    diff: params
                        .get("patch")
                        .or_else(|| params.get("changes"))
                        .cloned(),
                });
            }
        }
        "thread/compacted" => {
            sink.emit(AcpEvent::ContextCompacted {
                reason: first_text(params, &["reason", "trigger"]),
                before_tokens: params.get("beforeTokens").and_then(Value::as_u64),
                after_tokens: params.get("afterTokens").and_then(Value::as_u64),
            });
        }
        "thread/tokenUsage/updated" => {
            if let Some(usage) = params
                .get("tokenUsage")
                .and_then(|u| u.get("total"))
                .filter(|u| u.is_object())
            {
                let pick = |names: &[&str]| -> Option<u64> {
                    names
                        .iter()
                        .find_map(|name| usage.get(*name).and_then(Value::as_u64))
                };
                sink.emit(AcpEvent::Usage {
                    input_tokens: pick(&["inputTokens"]),
                    output_tokens: pick(&["outputTokens"]),
                    cache_read_tokens: pick(&["cachedInputTokens"]),
                    cache_write_tokens: pick(&["cacheWriteInputTokens"]),
                    reasoning_tokens: pick(&["reasoningOutputTokens"]),
                    total_tokens: pick(&["totalTokens"]),
                    context_window: params
                        .get("tokenUsage")
                        .and_then(|u| u.get("modelContextWindow"))
                        .and_then(Value::as_u64),
                    cost_usd: None,
                });
            }
        }
        // Thread/turn lifecycle notifications are control-plane metadata. They are not
        // assistant content and must not become raw debug cards in the conversation.
        _ if is_codex_lifecycle_notification(method) => {}
        // Advisories that must not abort the turn: rate/model reroutes, deprecations,
        // config problems. `error` stays out of this arm — it is handled as a failure.
        "warning" | "guardianWarning" | "deprecationNotice" | "configWarning"
        | "model/rerouted" => {
            if let Some(text) = first_text(params, &["message", "text", "reason", "detail"]) {
                sink.emit(AcpEvent::Notice {
                    level: if method == "deprecationNotice" {
                        "info".to_string()
                    } else {
                        "warning".to_string()
                    },
                    code: Some(method.to_string()),
                    text,
                });
            }
        }
        // Codex's plan surface. Routed to the plan panel rather than to one more tool card.
        "turn/plan/updated" => {
            if let Some(entries) = params
                .get("plan")
                .and_then(|p| p.get("items").or(Some(p)))
                .and_then(Value::as_array)
            {
                sink.emit(AcpEvent::Plan {
                    entries: entries.clone(),
                });
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
                return Err(extract_codex_turn_error(params, &status));
            }
            return Ok(true);
        }
        "error" => {
            return Err(extract_codex_turn_error(params, "error"));
        }
        _ if !method.is_empty() => {
            sink.emit(AcpEvent::Unknown {
                type_name: method.to_string(),
                raw: message.clone(),
            });
        }
        _ => {}
    }
    Ok(false)
}

fn codex_tool_status(item: &Value, method: &str) -> String {
    let raw = item
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match raw.as_str() {
        "inprogress" | "in_progress" | "running" | "pending" => "running".to_string(),
        "completed" | "success" => "completed".to_string(),
        "failed" | "error" => "failure".to_string(),
        "declined" | "cancelled" | "canceled" => "cancelled".to_string(),
        _ if method.ends_with("completed") => "completed".to_string(),
        _ => "running".to_string(),
    }
}

fn is_codex_lifecycle_notification(method: &str) -> bool {
    matches!(
        method,
        "account/rateLimits/updated"
            | "turn/diff/updated"
            | "thread/started"
            | "thread/resumed"
            | "thread/archived"
            | "turn/started"
    )
}

pub(super) fn extract_codex_turn_error(params: &Value, status: &str) -> String {
    if let Some(turn) = params.get("turn") {
        if let Some(err_val) = turn.get("error").or_else(|| turn.get("error_message")) {
            if let Some(msg) = extract_error_message(err_val) {
                return msg;
            }
        }
        if let Some(msg_val) = turn.get("message") {
            if let Some(msg) = extract_error_message(msg_val) {
                return msg;
            }
        }
    }

    if let Some(err_val) = params.get("error") {
        if let Some(msg) = extract_error_message(err_val) {
            return msg;
        }
    }
    if let Some(msg_val) = params.get("message") {
        if let Some(msg) = extract_error_message(msg_val) {
            return msg;
        }
    }

    if status.is_empty() {
        "Codex turn failed".to_string()
    } else {
        format!("Codex turn ended with status {status}")
    }
}

pub(super) fn extract_error_message(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(obj) = value.as_object() {
        let msg = obj
            .get("message")
            .or_else(|| obj.get("text"))
            .or_else(|| obj.get("detail"))
            .and_then(extract_error_message);
        let code = obj
            .get("code")
            .and_then(Value::as_str)
            .filter(|c| !c.trim().is_empty());
        match (code, msg) {
            (Some(c), Some(m)) if !m.contains(c) => return Some(format!("{c}: {m}")),
            (_, Some(m)) => return Some(m),
            (Some(c), None) => return Some(c.to_string()),
            (None, None) => {}
        }
        for key in ["error", "reason"] {
            if let Some(inner) = obj.get(key) {
                if let Some(msg) = extract_error_message(inner) {
                    return Some(msg);
                }
            }
        }
    }
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(msg) = extract_error_message(item) {
                return Some(msg);
            }
        }
    }
    None
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
    remember_runtime_models(runtime_key, models.iter().map(|m| m.id.clone()).collect());
    NativeCatalog {
        options: codex_options(&models),
        modes: vec![],
        session_id: String::new(),
    }
}

/// The service tier id the selected model advertises for fast mode, read back from the
/// discovered catalog so the turn parameter always matches what `model/list` actually said.
fn fast_tier_for_model(runtime_key: &str, model_id: &str) -> Option<String> {
    list_discovered_config_options(runtime_key)
        .iter()
        .find(|opt| opt.get("id").and_then(Value::as_str) == Some("model"))
        .and_then(|opt| opt.get("options").and_then(Value::as_array).cloned())?
        .iter()
        .find(|entry| entry.get("value").and_then(Value::as_str) == Some(model_id))
        .and_then(|entry| entry.get("fastTier").and_then(Value::as_str))
        .map(ToString::to_string)
}

/// One model as `model/list` describes it. Codex reports display names, per-model reasoning
/// efforts (with their own descriptions), the default effort and which model is the default —
/// all of which used to be discarded in favour of a hardcoded effort list that both invented
/// a `minimal` level and omitted the real `max` and `ultra` ones.
struct CodexModel {
    id: String,
    label: String,
    blurb: String,
    effort_levels: Vec<String>,
    effort_blurbs: Vec<(String, String)>,
    default_effort: Option<String>,
    is_default: bool,
    /// Service tier id to send as `turn/start`'s `serviceTier` for "fast mode", when the model
    /// advertises one. `model/list` reports these as `serviceTiers: [{id, name, description}]`
    /// alongside `additionalSpeedTiers`; the parameter is documented in Codex's own generated
    /// schema (`codex app-server generate-json-schema` → `TurnStartParams.serviceTier`).
    fast_tier: Option<String>,
}

fn extract_models(value: &Value) -> Option<Vec<CodexModel>> {
    let rows = value
        .get("data")
        .or_else(|| value.get("models"))
        .or_else(|| value.as_array().map(|_| value))
        .and_then(Value::as_array)?;
    let str_field = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
    };
    Some(
        rows.iter()
            // Codex marks models it does not want offered; showing them anyway hands the user
            // a choice the runtime will refuse.
            .filter(|row| row.get("hidden").and_then(Value::as_bool) != Some(true))
            .filter_map(|row| {
                let id = str_field(row, "id").or_else(|| str_field(row, "model"))?;
                let efforts: Vec<(String, String)> = row
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|entry| {
                                let level = entry
                                    .get("reasoningEffort")
                                    .and_then(Value::as_str)
                                    .or_else(|| entry.as_str())?;
                                let blurb = entry
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                Some((level.to_string(), blurb.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Only offer the toggle when the model actually advertises a faster tier, and
                // carry that tier's real id rather than assuming one.
                let fast_tier =
                    row.get("serviceTiers")
                        .and_then(Value::as_array)
                        .and_then(|tiers| {
                            tiers.iter().find_map(|tier| {
                                let id = tier.get("id").and_then(Value::as_str)?;
                                let name =
                                    tier.get("name").and_then(Value::as_str).unwrap_or_default();
                                (name.eq_ignore_ascii_case("fast")
                                    || id.eq_ignore_ascii_case("fast"))
                                .then(|| id.to_string())
                            })
                        });
                Some(CodexModel {
                    label: str_field(row, "displayName").unwrap_or_else(|| id.clone()),
                    blurb: str_field(row, "description").unwrap_or_default(),
                    effort_levels: efforts.iter().map(|(level, _)| level.clone()).collect(),
                    effort_blurbs: efforts,
                    default_effort: str_field(row, "defaultReasoningEffort"),
                    is_default: row.get("isDefault").and_then(Value::as_bool) == Some(true),
                    fast_tier,
                    id,
                })
            })
            .collect(),
    )
}

/// Codex's parameters, each declaring how its value reaches `turn/start`. The per-model
/// capability fields the picker reads (`effortLevels`, `defaultEffort`, `isDefault`,
/// `supportsFast`, `fastTier`) are merged onto the model values, since they describe a model
/// rather than the parameter itself.
fn codex_options(models: &[CodexModel]) -> Vec<Value> {
    let model_values = models
        .iter()
        .map(|m| {
            json!({
                "value": m.id,
                "name": m.label,
                "description": m.blurb,
                "effortLevels": m.effort_levels,
                "defaultEffort": m.default_effort,
                "isDefault": m.is_default,
                "oneMillion": false,
                "supportsFast": m.fast_tier.is_some(),
                "fastTier": m.fast_tier,
            })
        })
        .collect::<Vec<_>>();

    let model_spec = OptionSpec::select(
        "model",
        "Model",
        Wire::TurnParam {
            name: "model".to_string(),
            on_value: None,
        },
        Vec::new(),
    )
    .describe("Discovered from the native runtime")
    .defaulting_to(models.iter().find(|m| m.is_default).map(|m| m.id.clone()));
    let mut model_value = model_spec.to_value();
    // The model list carries per-model capabilities the generic `values` shape has no room
    // for, so it replaces both the new and the legacy value arrays.
    model_value["values"] = json!(model_values);
    model_value["options"] = json!(model_values);

    let mut options = vec![model_value];

    // Runtime-wide fallback for a model the listing did not describe: the union of every
    // effort Codex reported, carrying the descriptions it gave them.
    let mut seen = std::collections::BTreeSet::new();
    let mut effort_values = Vec::new();
    for model in models {
        for (level, blurb) in &model.effort_blurbs {
            if seen.insert(level.clone()) {
                effort_values.push(OptionValue::new(level, level).described(blurb));
            }
        }
    }
    if !effort_values.is_empty() {
        options.push(
            // Stored as `reasoning_effort`, delivered as `effort` — the exact mismatch the
            // old name-guessing list existed to paper over.
            OptionSpec::select(
                "reasoning_effort",
                "Effort",
                Wire::TurnParam {
                    name: "effort".to_string(),
                    on_value: None,
                },
                effort_values,
            )
            .defaulting_to(
                models
                    .iter()
                    .find(|m| m.is_default)
                    .and_then(|m| m.default_effort.clone()),
            )
            .to_value(),
        );
    }

    // Only offered where a model advertises a faster service tier, carrying that tier's real
    // id from `model/list` rather than an assumed one.
    let fast_models: Vec<String> = models
        .iter()
        .filter(|m| m.fast_tier.is_some())
        .map(|m| m.id.clone())
        .collect();
    if let Some(tier) = models.iter().find_map(|m| m.fast_tier.clone()) {
        options.push(
            OptionSpec::toggle(
                "fast",
                "Fast mode",
                Wire::TurnParam {
                    name: "serviceTier".to_string(),
                    on_value: Some(tier),
                },
            )
            .for_models(fast_models)
            .to_value(),
        );
    }

    options
}

fn extract_thread_id(value: &Value) -> Option<String> {
    value
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("threadId").and_then(Value::as_str))
        .map(ToString::to_string)
}

fn extract_turn_id(value: &Value) -> Option<String> {
    value
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("turnId").and_then(Value::as_str))
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim shape from `model/list` on codex 0.150.1 — the same response that showed the
    /// hardcoded effort list was both inventing `minimal` and omitting `max`/`ultra`.
    fn model_list_response() -> Value {
        json!({ "data": [
            {
                "id": "gpt-5.6-sol",
                "displayName": "GPT-5.6-Sol",
                "description": "Reliable agentic workhorse for everyday tasks.",
                "hidden": false,
                "supportedReasoningEfforts": [
                    { "reasoningEffort": "low", "description": "Fast responses with lighter reasoning" },
                    { "reasoningEffort": "max", "description": "Maximum reasoning depth" },
                    { "reasoningEffort": "ultra", "description": "Maximum reasoning with delegation" }
                ],
                "defaultReasoningEffort": "low",
                "additionalSpeedTiers": ["fast"],
                "serviceTiers": [
                    { "id": "priority", "name": "Fast", "description": "1.5x speed, increased usage" }
                ],
                "isDefault": true
            },
            { "id": "gpt-legacy", "displayName": "Legacy", "hidden": true },
            { "id": "gpt-5.5", "displayName": "GPT-5.5", "supportedReasoningEfforts": [] }
        ]})
    }

    #[test]
    fn lifecycle_notifications_do_not_become_conversation_blocks() {
        assert!(is_codex_lifecycle_notification(
            "account/rateLimits/updated"
        ));
        assert!(is_codex_lifecycle_notification("turn/diff/updated"));
        assert!(is_codex_lifecycle_notification("thread/started"));
        assert!(is_codex_lifecycle_notification("turn/started"));
        assert!(!is_codex_lifecycle_notification("item/agentMessage/delta"));
    }

    #[test]
    fn model_list_metadata_is_read_not_invented() {
        let models = extract_models(&model_list_response()).expect("parses");
        assert_eq!(models.len(), 2, "hidden models must not be offered");

        let sol = &models[0];
        assert_eq!(sol.label, "GPT-5.6-Sol");
        assert!(sol.blurb.starts_with("Reliable agentic"));
        assert_eq!(sol.effort_levels, vec!["low", "max", "ultra"]);
        assert_eq!(sol.default_effort.as_deref(), Some("low"));
        assert!(sol.is_default);
    }

    #[test]
    fn fast_mode_carries_the_tier_id_the_runtime_reported() {
        // The toggle must only exist where Codex advertises a tier, and must send that tier's
        // real id — `TurnStartParams.serviceTier` per codex's own generated JSON schema.
        let models = extract_models(&model_list_response()).expect("parses");
        assert_eq!(models[0].fast_tier.as_deref(), Some("priority"));
        assert_eq!(models[1].fast_tier, None);

        let options = codex_options(&models);
        let model_opt = options
            .iter()
            .find(|o| o["id"] == "model")
            .expect("model option");
        let entries = model_opt["options"].as_array().expect("entries");
        assert_eq!(entries[0]["supportsFast"], json!(true));
        assert_eq!(entries[0]["fastTier"], json!("priority"));
        assert_eq!(entries[1]["supportsFast"], json!(false));
    }

    #[test]
    fn effort_option_unions_the_reported_levels_with_their_descriptions() {
        let models = extract_models(&model_list_response()).expect("parses");
        let options = codex_options(&models);
        let effort = options
            .iter()
            .find(|o| o["id"] == "reasoning_effort")
            .expect("effort option");
        let values: Vec<&str> = effort["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["value"].as_str().unwrap())
            .collect();
        assert_eq!(values, vec!["low", "max", "ultra"]);
        assert!(!values.contains(&"minimal"), "invented level must be gone");
    }

    #[test]
    fn extract_codex_turn_error_nested_in_turn_error_object() {
        let params = json!({
            "turn": {
                "id": "turn-123",
                "status": "failed",
                "error": {
                    "code": "usage_limit_exceeded",
                    "message": "You've hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Sep 17th, 2026 12:55 AM."
                }
            }
        });
        let err = extract_codex_turn_error(&params, "failed");
        assert!(err.contains("usage_limit_exceeded"));
        assert!(err.contains("You've hit your usage limit"));
    }

    #[test]
    fn extract_codex_turn_error_string_in_turn() {
        let params = json!({
            "turn": {
                "status": "failed",
                "error": "Authentication token expired"
            }
        });
        let err = extract_codex_turn_error(&params, "failed");
        assert_eq!(err, "Authentication token expired");
    }

    #[test]
    fn extract_codex_turn_error_top_level_error() {
        let params = json!({
            "turn": { "status": "failed" },
            "error": { "message": "Connection reset" }
        });
        let err = extract_codex_turn_error(&params, "failed");
        assert_eq!(err, "Connection reset");
    }

    #[test]
    fn extract_codex_turn_error_fallback_to_status() {
        let params = json!({
            "turn": { "status": "interrupted" }
        });
        let err = extract_codex_turn_error(&params, "interrupted");
        assert_eq!(err, "Codex turn ended with status interrupted");
    }

    #[test]
    fn codex_item_status_preserves_failure_and_decline() {
        assert_eq!(
            codex_tool_status(&json!({"status": "failed"}), "item/completed"),
            "failure"
        );
        assert_eq!(
            codex_tool_status(&json!({"status": "declined"}), "item/completed"),
            "cancelled"
        );
        assert_eq!(
            codex_tool_status(&json!({"status": "inProgress"}), "item/started"),
            "running"
        );
    }
}
