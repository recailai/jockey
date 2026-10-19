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
    cancel_requested: bool,
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

/// A claimed turn slot. The session counts as busy from the moment this exists — before any
/// I/O — so two clients submitting at the same instant cannot both start a turn. Dropping it
/// without calling [`begin`] frees the slot again.
pub(crate) struct Reservation {
    slot: Option<(String, Arc<Mutex<ActiveTurn>>)>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some((sid, entry)) = self.slot.take() {
            release(&sid, &entry);
        }
    }
}

fn release(session_id: &str, entry: &Arc<Mutex<ActiveTurn>>) {
    active_turns().remove_if(session_id, |_, current| Arc::ptr_eq(current, entry));
}

/// Claim the session for a new turn; `None` when a turn is already running.
pub(crate) fn reserve(session_id: &str, assistant_message_id: Option<&str>) -> Option<Reservation> {
    use dashmap::mapref::entry::Entry;
    let turn_id = uuid::Uuid::new_v4().to_string();
    let message_id = non_empty_id(assistant_message_id).unwrap_or_else(|| turn_id.clone());
    match active_turns().entry(session_id.to_string()) {
        Entry::Occupied(_) => None,
        Entry::Vacant(slot) => {
            let entry = Arc::new(Mutex::new(ActiveTurn {
                turn_id,
                message_id,
                transcript: Transcript::new(""),
                dirty: false,
                finished: false,
                cancel_requested: false,
            }));
            slot.insert(entry.clone());
            Some(Reservation {
                slot: Some((session_id.to_string(), entry)),
            })
        }
    }
}

/// Mark the running turn as cancelled by a user, so its outcome is reported as a cancel
/// rather than a failure. Returns whether a turn was running.
pub(crate) fn mark_cancel_requested(session_id: &str) -> bool {
    let Some(entry) = active_turns().get(session_id).map(|e| e.value().clone()) else {
        return false;
    };
    if let Ok(mut turn) = entry.lock() {
        turn.cancel_requested = true;
    }
    true
}

pub(crate) struct TurnStart<'a> {
    pub(crate) role_label: &'a str,
    pub(crate) user_text: &'a str,
    pub(crate) images: &'a [ImageAttachment],
    /// Client-chosen id lets a view render optimistically and later match the engine's row.
    pub(crate) user_message_id: Option<&'a str>,
    /// Session fields the engine changed for this turn (routed persona, engine, auto-title),
    /// announced with `turn/started` so every view applies the same change.
    pub(crate) session_patch: Value,
}

pub(crate) struct TurnHandle {
    app: AppHandle,
    state: AppState,
    session_id: String,
    turn_id: String,
    entry: Arc<Mutex<ActiveTurn>>,
    persist_lock: Arc<Mutex<()>>,
}

fn non_empty_id(id: Option<&str>) -> Option<String> {
    id.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Persist the user's message, start recording, and announce the turn.
pub(crate) fn begin(
    app: &AppHandle,
    state: &AppState,
    mut reservation: Reservation,
    start: TurnStart<'_>,
) -> Result<TurnHandle, String> {
    let (session_id, entry) = reservation
        .slot
        .clone()
        .ok_or_else(|| "turn reservation already used".to_string())?;
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
    // On failure the reservation drops here and frees the slot.
    persist(state, &session_id, &user_message)?;
    reservation.slot = None;

    let (turn_id, message_id) = {
        let mut turn = entry
            .lock()
            .map_err(|_| "turn state poisoned".to_string())?;
        turn.transcript = Transcript::new(start.role_label);
        (turn.turn_id.clone(), turn.message_id.clone())
    };

    let persist_lock = Arc::new(Mutex::new(()));
    spawn_checkpoints(
        state.clone_refs(),
        session_id.clone(),
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
            "appSessionId": session_id,
            "turnId": turn_id,
            "assistantMessageId": message_id,
            "roleLabel": start.role_label,
            "userMessage": announced_user,
            "session": start.session_patch,
        }),
    );

    Ok(TurnHandle {
        app: app.clone(),
        state: state.clone_refs(),
        session_id,
        turn_id,
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

/// A turn task that panics or returns early must not leave the session busy forever.
impl Drop for TurnHandle {
    fn drop(&mut self) {
        let abandoned = self
            .entry
            .lock()
            .map(|mut t| !std::mem::replace(&mut t.finished, true))
            .unwrap_or(true);
        if !abandoned {
            return;
        }
        release(&self.session_id, &self.entry);
        let _ = self.app.emit(
            TURN_FINISHED_EVENT,
            json!({
                "appSessionId": self.session_id,
                "turnId": self.turn_id,
                "ok": false,
                "cancelled": false,
                "isCommand": false,
                "error": "turn aborted",
                "messages": [],
            }),
        );
    }
}

impl TurnHandle {
    /// Persist the turn's final messages, release the session, and announce the outcome.
    /// Returns the messages exactly as stored, for the caller to adopt.
    pub(crate) fn finish(self, outcome: TurnOutcome<'_>, error: Option<&str>) -> Vec<Value> {
        let ok = matches!(outcome, TurnOutcome::Command { .. })
            || matches!(outcome, TurnOutcome::Reply { ok: true, .. });
        let is_command = matches!(outcome, TurnOutcome::Command { .. });
        let _write = self.persist_lock.lock();
        let (messages, cancelled) = match self.entry.lock() {
            Ok(mut turn) => {
                turn.finished = true;
                let messages = turn
                    .transcript
                    .finish(&turn.message_id, crate::now_ms(), &outcome);
                (messages, turn.cancel_requested && !ok)
            }
            Err(_) => (Vec::new(), false),
        };
        for msg in &messages {
            if let Err(e) = persist(&self.state, &self.session_id, msg) {
                eprintln!("[engine] failed to persist turn message: {e}");
            }
        }
        release(&self.session_id, &self.entry);
        let _ = self.app.emit(
            TURN_FINISHED_EVENT,
            json!({
                "appSessionId": self.session_id,
                "turnId": self.turn_id,
                "ok": ok,
                "cancelled": cancelled,
                "isCommand": is_command,
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
            cancel_requested: false,
        }));
        active_turns().insert(sid.to_string(), entry.clone());
        entry
    }

    #[test]
    fn reservation_is_exclusive_and_freed_on_drop() {
        let first = reserve("reserve-a", Some("m1")).expect("idle session reserves");
        assert!(is_busy("reserve-a"));
        assert!(
            reserve("reserve-a", None).is_none(),
            "second reservation must fail"
        );
        drop(first);
        assert!(
            !is_busy("reserve-a"),
            "an unused reservation frees the slot"
        );
        let again = reserve("reserve-a", None).expect("slot is free again");
        drop(again);
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
