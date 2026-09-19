use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::db::app_session::{close_app_session_internal, create_app_session_internal};
use crate::db::with_db;
use crate::types::AppState;

pub(crate) fn list_sessions(state: &AppState, params: Value) -> Result<Value, String> {
    let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
    let sessions = with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, created_at, last_active_at \
                 FROM app_sessions WHERE closed_at IS NULL ORDER BY last_active_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<Value> = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, Option<String>>(1)?,
                    "activeRole": row.get::<_, Option<String>>(2)?,
                    "runtimeKind": row.get::<_, Option<String>>(3)?,
                    "runtimeProfileId": row.get::<_, Option<String>>(4)?,
                    "cwd": row.get::<_, Option<String>>(5)?,
                    "createdAt": row.get::<_, i64>(6)?,
                    "lastActiveAt": row.get::<_, i64>(7)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    })?;
    Ok(json!(sessions))
}

pub(crate) fn get_session(state: &AppState, params: Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id is required")?;
    let session = with_db(state, |conn| {
        conn.query_row(
            "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, created_at, last_active_at \
             FROM app_sessions WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, Option<String>>(1)?,
                    "activeRole": row.get::<_, Option<String>>(2)?,
                    "runtimeKind": row.get::<_, Option<String>>(3)?,
                    "runtimeProfileId": row.get::<_, Option<String>>(4)?,
                    "cwd": row.get::<_, Option<String>>(5)?,
                    "createdAt": row.get::<_, i64>(6)?,
                    "lastActiveAt": row.get::<_, i64>(7)?,
                }))
            },
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(session)
}

pub(crate) fn update_session(state: &AppState, params: Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id is required")?;
    let now = crate::now_ms();
    with_db(state, |conn| {
        if let Some(title) = params.get("title").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE app_sessions SET title = ?1, last_active_at = ?2 WHERE id = ?3",
                rusqlite::params![title, now, id],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(role) = params.get("activeRole").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE app_sessions SET active_role = ?1, last_active_at = ?2 WHERE id = ?3",
                rusqlite::params![role, now, id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })?;
    Ok(json!(format!("Session '{id}' updated")))
}

pub(crate) fn create_session(state: &AppState, params: Value) -> Result<Value, String> {
    let title = params.get("title").and_then(|v| v.as_str());
    let project_id = params
        .get("projectId")
        .or_else(|| params.get("project_id"))
        .and_then(|v| v.as_str());
    let runtime_kind = params
        .get("runtimeKind")
        .or_else(|| params.get("runtime_kind"))
        .and_then(|v| v.as_str());
    let session = create_app_session_internal(state, title, project_id, runtime_kind, None, None)?;
    Ok(json!({
        "id": session.id,
        "title": session.title,
        "activeRole": session.active_role,
    }))
}

pub(crate) fn close_session(state: &AppState, params: Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id is required")?;
    close_app_session_internal(state, id)?;
    Ok(json!(format!("Session '{id}' closed")))
}

const DEFAULT_CONTEXT_LIMIT: i64 = 5;
const MAX_CONTEXT_LIMIT: i64 = 50;
const MAX_CONTEXT_SCAN: i64 = 100;

#[derive(Debug, Clone)]
struct SessionMessageRow {
    id: i64,
    role_name: String,
    content: String,
    content_type: String,
    payload: Option<Value>,
    created_at: i64,
}

fn bounded_context_limit(params: &Value) -> i64 {
    params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_CONTEXT_LIMIT)
        .clamp(1, MAX_CONTEXT_LIMIT)
}

fn requested_set(params: &Value, key: &str) -> Option<HashSet<String>> {
    params.get(key).and_then(Value::as_array).map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect()
    })
}

fn validate_requested_values(params: &Value, key: &str, allowed: &[&str]) -> Result<(), String> {
    if let Some(values) = params.get(key).and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            if !allowed
                .iter()
                .any(|item| item.eq_ignore_ascii_case(value.trim()))
            {
                return Err(format!(
                    "{key} contains unsupported value '{value}'; allowed values: {}",
                    allowed.join(", ")
                ));
            }
        }
    }
    Ok(())
}

fn session_id_from(params: &Value) -> Result<&str, String> {
    params
        .get("appSessionId")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "appSessionId is required".to_string())
}

fn message_type(row: &SessionMessageRow) -> &'static str {
    if row.role_name.eq_ignore_ascii_case("user") {
        return "user";
    }
    if row.role_name.eq_ignore_ascii_case("event") {
        return "event";
    }
    if row.role_name.eq_ignore_ascii_case("tool") {
        return "tool";
    }
    if row.role_name.eq_ignore_ascii_case("thought")
        || row.role_name.eq_ignore_ascii_case("thinking")
    {
        return "thought";
    }
    if row
        .payload
        .as_ref()
        .and_then(|payload| {
            payload
                .get("toolCalls")
                .or_else(|| payload.get("tool_calls"))
                .or_else(|| payload.get("tools"))
        })
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty())
    {
        return "tool";
    }
    if row.payload.as_ref().is_some_and(|payload| {
        payload.get("thoughtText").is_some() || payload.get("thought").is_some()
    }) {
        return "thought";
    }
    "assistant"
}

fn message_purpose(message_type: &str) -> &'static str {
    match message_type {
        "user" => "request",
        "event" => "providerEvent",
        "tool" => "toolExecution",
        "thought" => "reasoning",
        _ => "response",
    }
}

fn row_matches(
    row: &SessionMessageRow,
    role_name: Option<&str>,
    message_types: Option<&HashSet<String>>,
) -> bool {
    if role_name.is_some_and(|role| row.role_name != role) {
        return false;
    }
    message_types.is_none_or(|allowed| allowed.contains(message_type(row)))
}

fn tool_summary(payload: &Value) -> Option<Value> {
    let calls = payload
        .get("toolCalls")
        .or_else(|| payload.get("tool_calls"))
        .or_else(|| payload.get("tools"))?
        .as_array()?;
    if calls.is_empty() {
        return None;
    }
    let items: Vec<Value> = calls
        .iter()
        .filter_map(|call| {
            let object = call.as_object()?;
            Some(json!({
                "toolCallId": object.get("toolCallId").or_else(|| object.get("id")),
                "name": object.get("name").or_else(|| object.get("kind")),
                "status": object.get("status"),
                "title": object.get("title"),
            }))
        })
        .collect();
    Some(json!(items))
}

fn message_json(row: &SessionMessageRow, include: &HashSet<String>) -> Value {
    let kind = message_type(row);
    let mut message = json!({
        "id": row.id,
        "roleName": row.role_name,
        "messageType": kind,
        "purpose": message_purpose(kind),
        "contentType": row.content_type,
        "createdAt": row.created_at,
    });
    if include.contains("text") {
        message["content"] = json!(row.content);
    }
    if let Some(payload) = row.payload.as_ref() {
        if include.contains("toolSummary") {
            if let Some(summary) = tool_summary(payload) {
                message["toolSummary"] = summary;
            }
        }
        if include.contains("tools") {
            if let Some(tools) = payload
                .get("toolCalls")
                .or_else(|| payload.get("tool_calls"))
                .or_else(|| payload.get("tools"))
            {
                message["toolCalls"] = tools.clone();
            }
        }
        if include.contains("toolOutput") {
            for key in [
                "toolOutput",
                "toolOutputs",
                "tool_output",
                "tool_outputs",
                "output",
            ] {
                if let Some(output) = payload.get(key) {
                    message["toolOutput"] = output.clone();
                    break;
                }
            }
        }
        if include.contains("raw") {
            message["payload"] = payload.clone();
        }
    }
    message
}

fn load_message_rows(
    state: &AppState,
    session_id: &str,
    order: &str,
    cursor: Option<i64>,
) -> Result<Vec<SessionMessageRow>, String> {
    with_db(state, |conn| {
        let (comparison, ordering) = if order == "oldest" {
            (">", "ASC")
        } else {
            ("<", "DESC")
        };
        let sql = if cursor.is_some() {
            format!(
                "SELECT id, role_name, content, content_type, payload, created_at
                 FROM app_session_messages WHERE session_id = ?1 AND id {comparison} ?2
                 ORDER BY id {ordering} LIMIT ?3"
            )
        } else {
            format!(
                "SELECT id, role_name, content, content_type, payload, created_at
                 FROM app_session_messages WHERE session_id = ?1
                 ORDER BY id {ordering} LIMIT ?2"
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        if let Some(cursor) = cursor {
            let rows = stmt
                .query_map(
                    rusqlite::params![session_id, cursor, MAX_CONTEXT_SCAN],
                    |row| {
                        Ok(SessionMessageRow {
                            id: row.get(0)?,
                            role_name: row.get(1)?,
                            content: row.get(2)?,
                            content_type: row.get(3)?,
                            payload: row
                                .get::<_, Option<String>>(4)?
                                .and_then(|raw| serde_json::from_str(&raw).ok()),
                            created_at: row.get(5)?,
                        })
                    },
                )
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        } else {
            let rows = stmt
                .query_map(rusqlite::params![session_id, MAX_CONTEXT_SCAN], |row| {
                    Ok(SessionMessageRow {
                        id: row.get(0)?,
                        role_name: row.get(1)?,
                        content: row.get(2)?,
                        content_type: row.get(3)?,
                        payload: row
                            .get::<_, Option<String>>(4)?
                            .and_then(|raw| serde_json::from_str(&raw).ok()),
                        created_at: row.get(5)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
    })
}

fn load_session_meta(state: &AppState, session_id: &str) -> Result<Value, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, title, active_role, runtime_kind, runtime_profile_id, cwd, project_id, created_at, last_active_at
             FROM app_sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, Option<String>>(1)?,
                    "activeRole": row.get::<_, Option<String>>(2)?,
                    "runtimeKind": row.get::<_, Option<String>>(3)?,
                    "runtimeProfileId": row.get::<_, Option<String>>(4)?,
                    "cwd": row.get::<_, Option<String>>(5)?,
                    "projectId": row.get::<_, Option<String>>(6)?,
                    "createdAt": row.get::<_, i64>(7)?,
                    "lastActiveAt": row.get::<_, i64>(8)?,
                }))
            },
        )
        .map_err(|error| error.to_string())
    })
}

fn build_roles(rows: &[SessionMessageRow]) -> Vec<Value> {
    let mut roles = std::collections::BTreeMap::<String, (usize, i64)>::new();
    for row in rows {
        if row.role_name.eq_ignore_ascii_case("user") || row.role_name.eq_ignore_ascii_case("event")
        {
            continue;
        }
        let entry = roles.entry(row.role_name.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(row.created_at);
    }
    roles
        .into_iter()
        .map(|(role_name, (message_count, last_message_at))| {
            json!({
                "roleName": role_name,
                "messageCount": message_count,
                "lastMessageAt": last_message_at,
            })
        })
        .collect()
}

fn build_turns(rows: &[&SessionMessageRow], include: &HashSet<String>) -> Vec<Value> {
    let mut chronological = rows.to_vec();
    chronological.sort_by_key(|row| row.id);
    let mut turns: Vec<Vec<&SessionMessageRow>> = Vec::new();
    for row in chronological {
        if row.role_name.eq_ignore_ascii_case("user") || turns.is_empty() {
            turns.push(Vec::new());
        }
        turns.last_mut().expect("turn exists").push(row);
    }
    turns
        .into_iter()
        .map(|turn| {
            let messages: Vec<Value> = turn.iter().map(|row| message_json(row, include)).collect();
            let role_names: Vec<String> = turn
                .iter()
                .map(|row| row.role_name.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            json!({
                "turnId": format!("turn:{}", turn.last().map(|row| row.id).unwrap_or_default()),
                "roleNames": role_names,
                "messageCount": messages.len(),
                "messages": messages,
            })
        })
        .collect()
}

pub(crate) fn get_session_context(state: &AppState, params: Value) -> Result<Value, String> {
    let session_id = session_id_from(&params)?;
    if session_id.chars().count() > 256 {
        return Err("appSessionId is too long (maximum 256 characters)".to_string());
    }
    let view = params
        .get("view")
        .and_then(Value::as_str)
        .unwrap_or("summary");
    if !matches!(view, "summary" | "roles" | "turns" | "messages") {
        return Err("view must be one of: summary, roles, turns, messages".to_string());
    }
    let order = params
        .get("order")
        .and_then(Value::as_str)
        .unwrap_or("latest");
    if !matches!(order, "latest" | "oldest") {
        return Err("order must be latest or oldest".to_string());
    }
    let role_name = params.get("roleName").and_then(Value::as_str);
    if role_name.is_some_and(|role| role.chars().count() > 128) {
        return Err("roleName is too long (maximum 128 characters)".to_string());
    }
    validate_requested_values(
        &params,
        "messageTypes",
        &["user", "assistant", "tool", "thought", "event"],
    )?;
    validate_requested_values(
        &params,
        "include",
        &["text", "toolSummary", "tools", "toolOutput", "raw"],
    )?;
    let message_types = requested_set(&params, "messageTypes");
    let mut include = requested_set(&params, "include").unwrap_or_else(|| {
        ["text", "toolSummary"]
            .into_iter()
            .map(str::to_string)
            .collect()
    });
    if params
        .get("includePayload")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        include.insert("raw".to_string());
    }
    let cursor = params.get("cursor").and_then(Value::as_i64);
    let session = load_session_meta(state, session_id)?;
    let rows = load_message_rows(state, session_id, order, cursor)?;
    let filtered: Vec<&SessionMessageRow> = rows
        .iter()
        .filter(|row| row_matches(row, role_name, message_types.as_ref()))
        .take(bounded_context_limit(&params) as usize)
        .collect();
    let messages: Vec<Value> = filtered
        .iter()
        .map(|row| message_json(row, &include))
        .collect();
    let all_roles = build_roles(&rows);
    let turns = build_turns(&filtered, &include);
    let mut output = Map::new();
    output.insert("session".to_string(), session);
    output.insert("roles".to_string(), json!(all_roles));
    match view {
        "roles" => {}
        "messages" => {
            output.insert("messages".to_string(), json!(messages));
            if let Some(last) = filtered.last() {
                output.insert("nextCursor".to_string(), json!(last.id));
            }
        }
        "turns" => {
            output.insert("turns".to_string(), json!(turns));
        }
        _ => {
            output.insert("messages".to_string(), json!(messages));
            output.insert("turns".to_string(), json!(turns));
        }
    };
    Ok(Value::Object(output))
}

pub(crate) fn get_session_history(state: &AppState, params: Value) -> Result<Value, String> {
    let mut object = params.as_object().cloned().unwrap_or_default();
    object.insert("view".to_string(), json!("messages"));
    object.insert("order".to_string(), json!("oldest"));
    let result = get_session_context(state, Value::Object(object))?;
    Ok(result.get("messages").cloned().unwrap_or_else(|| json!([])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, role_name: &str, content: &str, payload: Option<Value>) -> SessionMessageRow {
        SessionMessageRow {
            id,
            role_name: role_name.to_string(),
            content: content.to_string(),
            content_type: "text".to_string(),
            payload,
            created_at: id,
        }
    }

    #[test]
    fn classifies_structured_tool_and_thought_messages() {
        let tool = row(
            1,
            "Developer",
            "",
            Some(json!({ "tool_calls": [{ "id": "call-1", "name": "shell" }] })),
        );
        let thought = row(2, "Developer", "", Some(json!({ "thoughtText": "plan" })));
        assert_eq!(message_type(&tool), "tool");
        assert_eq!(message_type(&thought), "thought");
    }

    #[test]
    fn groups_adjacent_messages_at_user_boundaries() {
        let messages = vec![
            row(1, "user", "first", None),
            row(2, "Developer", "first answer", None),
            row(
                3,
                "Developer",
                "first tool",
                Some(json!({ "toolCalls": [{}] })),
            ),
            row(4, "user", "second", None),
            row(5, "Reviewer", "second answer", None),
        ];
        let refs: Vec<&SessionMessageRow> = messages.iter().rev().collect();
        let include = ["text".to_string()].into_iter().collect();
        let turns = build_turns(&refs, &include);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0]["messageCount"], 3);
        assert_eq!(turns[1]["messageCount"], 2);
        assert_eq!(turns[0]["messages"][0]["messageType"], "user");
    }

    #[test]
    fn rejects_unknown_message_fields() {
        let params = json!({ "messageTypes": ["providerEvent"] });
        assert!(validate_requested_values(
            &params,
            "messageTypes",
            &["user", "assistant", "tool", "thought", "event"]
        )
        .is_err());
    }
}
