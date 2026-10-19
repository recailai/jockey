//! Turn lifecycle owned by the engine: every turn — whichever client submitted it — is
//! recorded, checkpointed, persisted and announced the same way.
//!
//! The recorder taps the engine's own `acp/delta` / `acp/stream` emissions instead of being
//! threaded through every adapter's emit site; those two events are already the one stream
//! all runtimes funnel into, and Rust listeners run synchronously in emit order.

use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener};

use super::transcript::{storage_columns, Transcript, TurnOutcome};
use crate::types::{AppState, ImageAttachment};

pub(crate) const TURN_STARTED_EVENT: &str = "turn/started";
pub(crate) const TURN_FINISHED_EVENT: &str = "turn/finished";
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(1500);

struct ActiveTurn {
    turn_id: String,
    message_id: String,
    transcript: Transcript,
    dirty: bool,
    finished: bool,
}

fn active_turns() -> &'static DashMap<String, Arc<Mutex<ActiveTurn>>> {
    static TURNS: OnceLock<DashMap<String, Arc<Mutex<ActiveTurn>>>> = OnceLock::new();
    TURNS.get_or_init(DashMap::new)
}

/// Whether the engine is currently running a turn in this session.
pub(crate) fn is_busy(app_session_id: &str) -> bool {
    active_turns().contains_key(app_session_id)
}

/// Install the transcript taps. Called once from app setup.
pub(crate) fn install(app: &AppHandle) {
    app.listen_any("acp/delta", |ev| route(ev.payload(), true));
    app.listen_any("acp/stream", |ev| route(ev.payload(), false));
}

fn route(raw: &str, is_delta: bool) {
    let Ok(payload) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let Some(sid) = payload.get("appSessionId").and_then(Value::as_str) else {
        return;
    };
    let Some(entry) = active_turns().get(sid).map(|e| e.value().clone()) else {
        return;
    };
    let Ok(mut turn) = entry.lock() else {
        return;
    };
    if turn.finished {
        return;
    }
    // Frames are attributed by session, not by the adapter's `turnId`: adapters disagree on
    // what that id means (the permission bridge reuses the session id), so it cannot tell
    // one engine turn from the next.
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    let changed = if is_delta {
        payload
            .get("delta")
            .and_then(Value::as_str)
            .is_some_and(|d| turn.transcript.apply_text(d, role))
    } else {
        payload
            .get("event")
            .is_some_and(|e| turn.transcript.apply_event(e, role))
    };
    if changed {
        turn.dirty = true;
    }
}

fn persist(state: &AppState, session_id: &str, msg: &Value) -> Result<(), String> {
    let (content_type, payload) = storage_columns(msg);
    crate::db::app_session::save_app_message_internal(
        state,
        session_id,
        msg.get("id").and_then(Value::as_str),
        msg.get("roleName")
            .and_then(Value::as_str)
            .unwrap_or("assistant"),
        msg.get("text").and_then(Value::as_str).unwrap_or(""),
        content_type,
        payload.as_deref(),
    )
}

pub(crate) struct TurnStart<'a> {
    pub(crate) app_session_id: &'a str,
    pub(crate) role_label: &'a str,
    pub(crate) user_text: &'a str,
    pub(crate) images: &'a [ImageAttachment],
    /// Client-chosen ids let a view render optimistically and later match the engine's rows.
    pub(crate) user_message_id: Option<&'a str>,
    pub(crate) assistant_message_id: Option<&'a str>,
}

pub(crate) struct TurnHandle {
    app: AppHandle,
    state: AppState,
    session_id: String,
    turn_id: String,
    user_message: Value,
    entry: Arc<Mutex<ActiveTurn>>,
    persist_lock: Arc<Mutex<()>>,
}

fn non_empty_id(id: Option<&str>) -> Option<String> {
    id.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Persist the user's message, register the turn, and announce it.
pub(crate) fn begin(
    app: &AppHandle,
    state: &AppState,
    start: TurnStart<'_>,
) -> Result<TurnHandle, String> {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let message_id = non_empty_id(start.assistant_message_id).unwrap_or_else(|| turn_id.clone());
    let now = crate::now_ms();

    let mut user_message = json!({
        "id": non_empty_id(start.user_message_id).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        "roleName": "user",
        "text": start.user_text,
        "at": now,
    });
    if !start.images.is_empty() {
        user_message["images"] = json!(start
            .images
            .iter()
            .map(|a| json!({ "data": a.data, "mimeType": a.mime_type }))
            .collect::<Vec<_>>());
    }
    persist(state, start.app_session_id, &user_message)?;

    let entry = Arc::new(Mutex::new(ActiveTurn {
        turn_id: turn_id.clone(),
        message_id,
        transcript: Transcript::new(start.role_label),
        dirty: false,
        finished: false,
    }));
    active_turns().insert(start.app_session_id.to_string(), entry.clone());

    let persist_lock = Arc::new(Mutex::new(()));
    spawn_checkpoints(
        state.clone_refs(),
        start.app_session_id.to_string(),
        entry.clone(),
        persist_lock.clone(),
    );

    // The user message's image payloads can be megabytes; listeners only need to know it exists.
    let mut announced_user = user_message.clone();
    if let Some(images) = announced_user.get_mut("images") {
        *images = json!(images.as_array().map(|a| a.len()).unwrap_or(0));
    }
    let _ = app.emit(
        TURN_STARTED_EVENT,
        json!({
            "appSessionId": start.app_session_id,
            "turnId": turn_id,
            "roleLabel": start.role_label,
            "userMessage": announced_user,
        }),
    );

    Ok(TurnHandle {
        app: app.clone(),
        state: state.clone_refs(),
        session_id: start.app_session_id.to_string(),
        turn_id,
        user_message,
        entry,
        persist_lock,
    })
}

/// Periodic snapshot of the in-flight message so a crash or restart mid-turn keeps what has
/// streamed so far.
///
/// The turn lock is only held to take the snapshot: `route` runs inside Tauri's emit while
/// Tauri holds its listener lock, and a slow `route` makes concurrent emits from other
/// threads queue up. Writes are ordered by `persist_lock` instead, which `finish` also takes,
/// so a checkpoint can never land after (and clobber) the final write.
fn spawn_checkpoints(
    state: AppState,
    session_id: String,
    entry: Arc<Mutex<ActiveTurn>>,
    persist_lock: Arc<Mutex<()>>,
) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(CHECKPOINT_INTERVAL);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Ok(_write) = persist_lock.lock() else {
                return;
            };
            let snapshot = {
                let Ok(mut turn) = entry.lock() else { return };
                if turn.finished {
                    return;
                }
                if !turn.dirty || !turn.transcript.has_content() {
                    continue;
                }
                turn.dirty = false;
                turn.transcript
                    .message(&turn.message_id, crate::now_ms(), None)
            };
            if let Err(e) = persist(&state, &session_id, &snapshot) {
                eprintln!("[engine] checkpoint failed for {session_id}: {e}");
            }
        }
    });
}

impl TurnHandle {
    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub(crate) fn user_message(&self) -> &Value {
        &self.user_message
    }

    /// Persist the turn's final messages, release the session, and announce the outcome.
    /// Returns the messages exactly as stored, for the caller to adopt.
    pub(crate) fn finish(self, outcome: TurnOutcome<'_>, error: Option<&str>) -> Vec<Value> {
        let ok = matches!(outcome, TurnOutcome::Command { .. })
            || matches!(outcome, TurnOutcome::Reply { ok: true, .. });
        let _write = self.persist_lock.lock();
        let messages = match self.entry.lock() {
            Ok(mut turn) => {
                turn.finished = true;
                turn.transcript
                    .finish(&turn.message_id, crate::now_ms(), &outcome)
            }
            Err(_) => Vec::new(),
        };
        for msg in &messages {
            if let Err(e) = persist(&self.state, &self.session_id, msg) {
                eprintln!("[engine] failed to persist turn message: {e}");
            }
        }
        // A newer turn may already own the slot (cancel + immediate resend); only release ours.
        active_turns().remove_if(&self.session_id, |_, current| {
            current
                .lock()
                .map(|t| t.turn_id == self.turn_id)
                .unwrap_or(true)
        });
        let _ = self.app.emit(
            TURN_FINISHED_EVENT,
            json!({
                "appSessionId": self.session_id,
                "turnId": self.turn_id,
                "ok": ok,
                "error": error,
                "messages": messages,
            }),
        );
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(sid: &str, role: &str) -> Arc<Mutex<ActiveTurn>> {
        let entry = Arc::new(Mutex::new(ActiveTurn {
            turn_id: format!("turn-{sid}"),
            message_id: format!("msg-{sid}"),
            transcript: Transcript::new(role),
            dirty: false,
            finished: false,
        }));
        active_turns().insert(sid.to_string(), entry.clone());
        entry
    }

    #[test]
    fn frames_route_to_the_sessions_active_turn_only() {
        let entry = register("route-a", "Developer");
        route(
            &json!({"appSessionId": "route-a", "role": "Developer", "delta": "hi "}).to_string(),
            true,
        );
        route(
            &json!({"appSessionId": "route-a", "role": "Developer",
                    "event": {"kind": "toolCall", "toolCallId": "t1", "title": "Read"}})
            .to_string(),
            false,
        );
        route(
            &json!({"appSessionId": "someone-else", "role": "Developer", "delta": "nope"})
                .to_string(),
            true,
        );
        route("not json", true);

        let turn = entry.lock().unwrap();
        assert!(turn.dirty);
        let msg = turn.transcript.message(&turn.message_id, 1, None);
        assert_eq!(msg["id"], "msg-route-a");
        assert_eq!(msg["text"], "hi ");
        assert_eq!(msg["toolCalls"][0]["toolCallId"], "t1");
        drop(turn);
        assert!(is_busy("route-a"));
        assert!(!is_busy("someone-else"));
        active_turns().remove("route-a");
    }

    #[test]
    fn finished_turns_ignore_late_frames() {
        let entry = register("route-b", "Developer");
        entry.lock().unwrap().finished = true;
        route(
            &json!({"appSessionId": "route-b", "role": "Developer", "delta": "late"}).to_string(),
            true,
        );
        assert!(!entry.lock().unwrap().transcript.has_content());
        active_turns().remove("route-b");
    }
}
