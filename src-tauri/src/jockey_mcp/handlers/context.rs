use serde_json::{json, Value};

use crate::db::context::{
    clear_shared_context_internal, list_shared_context_internal, set_shared_context_internal,
};
use crate::types::AppState;

pub(crate) fn set_shared_context(state: &AppState, params: Value) -> Result<Value, String> {
    let scope = params
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or("key is required")?;
    let value = params
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or("value is required")?;
    set_shared_context_internal(state, scope, key, value)?;
    Ok(json!(format!("Context set: [{scope}] {key}")))
}

pub(crate) fn get_shared_context(state: &AppState, params: Value) -> Result<Value, String> {
    let scope = params
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let entries = list_shared_context_internal(state, scope)?;
    let out: Vec<Value> = entries
        .iter()
        .map(|e| json!({ "key": e.key, "value": e.value }))
        .collect();
    Ok(json!(out))
}

pub(crate) fn delete_shared_context(state: &AppState, params: Value) -> Result<Value, String> {
    let scope = params
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or("key is required")?;
    clear_shared_context_internal(state, scope, key)?;
    Ok(json!(format!("Context deleted: [{scope}] {key}")))
}

pub(crate) fn get_role_context(state: &AppState, params: Value) -> Result<Value, String> {
    let app_session_id = params
        .get("appSessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("appSessionId is required")?;
    let role_name = params.get("roleName").and_then(Value::as_str);
    let include_current_role = params
        .get("includeCurrentRole")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let requested_limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, 8);
    let mut query = serde_json::Map::new();
    query.insert("appSessionId".to_string(), json!(app_session_id));
    query.insert("view".to_string(), json!("turns"));
    query.insert(
        "limit".to_string(),
        json!(if role_name.is_some() && !include_current_role {
            50
        } else {
            requested_limit
        }),
    );
    query.insert("include".to_string(), json!(["text", "toolSummary"]));
    let mut result = super::sessions::get_session_context(state, Value::Object(query))?;
    let purpose = match params.get("purpose").and_then(Value::as_str) {
        Some("conversationHistory") => "conversationHistory",
        Some("taskState") => "taskState",
        _ => "roleHandoff",
    };
    result["sessionId"] = json!(app_session_id);
    result["roleName"] = role_name.map_or(Value::Null, |value| json!(value));
    result["purpose"] = json!(purpose);
    result["count"] = json!(result
        .get("turns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0));
    if include_current_role || role_name.is_none() {
        return Ok(result);
    }
    let Some(turns) = result.get("turns").and_then(Value::as_array) else {
        return Ok(result);
    };
    let filtered: Vec<Value> = turns
        .iter()
        .filter_map(|turn| {
            let messages = turn.get("messages")?.as_array()?;
            let kept_messages: Vec<Value> = messages
                .iter()
                .filter(|message| message.get("roleName").and_then(Value::as_str) != role_name)
                .cloned()
                .collect();
            if kept_messages.is_empty() {
                return None;
            }
            let mut turn = turn.clone();
            turn["messages"] = json!(kept_messages);
            turn["messageCount"] = json!(turn["messages"].as_array().map(Vec::len).unwrap_or(0));
            turn["roleNames"] = json!(turn["messages"]
                .as_array()
                .map(|messages| {
                    messages
                        .iter()
                        .filter_map(|message| message.get("roleName").and_then(Value::as_str))
                        .collect::<std::collections::BTreeSet<_>>()
                })
                .unwrap_or_default());
            Some(turn)
        })
        .take(requested_limit as usize)
        .collect();
    result["turns"] = json!(filtered);
    result["count"] = json!(result
        .get("turns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0));
    Ok(result)
}
