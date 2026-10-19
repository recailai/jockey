//! The engine's front door. Every client — desktop window, web companion, anything else —
//! submits input, cancels, and manages the queue through these functions, and learns what
//! happened from `turn/*` and `queue/changed` events. Nothing here knows which client called.

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::queue::{self, Batch};
use super::sessions;
use super::transcript::TurnOutcome;
use super::turns::{self, Reservation, TurnStart};
use crate::types::{AppState, ImageAttachment, InboxMessage};

pub(crate) const MESSAGE_ADDED_EVENT: &str = super::events::MESSAGE_ADDED;

#[derive(Clone, Default)]
pub(crate) struct SubmitInput {
    pub(crate) session_id: String,
    pub(crate) text: String,
    pub(crate) attachments: Vec<ImageAttachment>,
    /// Ids the caller already uses for its optimistic copies, so stored rows line up with them.
    pub(crate) user_message_id: Option<String>,
    pub(crate) assistant_message_id: Option<String>,
    /// The engine selected in the caller's composer; otherwise the session's own.
    pub(crate) runtime_kind: Option<String>,
}

#[derive(Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SubmitResult {
    /// A turn began; it is announced with `turn/started`.
    Started {},
    Queued {
        item: InboxMessage,
    },
    /// An agent control command (`/plan`, `/act`, `/auto`, `/cancel`) ran immediately.
    Control {
        message: String,
    },
}

struct PendingTurn {
    reservation: Reservation,
    text: String,
    attachments: Vec<ImageAttachment>,
    user_message_id: Option<String>,
    runtime_kind: Option<String>,
    /// Set when the turn came out of the queue; consumed once the turn has begun.
    batch: Option<Batch>,
}

fn control_command(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('/')?;
    let word = rest.split(|c: char| !c.is_ascii_alphanumeric()).next()?;
    matches!(word, "plan" | "act" | "auto" | "cancel").then_some(word)
}

fn load_head(
    state: &AppState,
    session_id: &str,
) -> Result<crate::db::app_session::SessionHead, String> {
    let head = crate::db::app_session::load_session_head(state, session_id)?
        .ok_or_else(|| format!("session not found: {session_id}"))?;
    if head.closed {
        return Err("session is closed; reopen it first".to_string());
    }
    Ok(head)
}

/// Submit input to a session: starts a turn when idle, queues it when busy.
pub(crate) async fn submit(app: &AppHandle, input: SubmitInput) -> Result<SubmitResult, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() {
        return Err("empty input".to_string());
    }
    let state = app.state::<AppState>();
    let head = load_head(&state, &input.session_id)?;

    if let Some(cmd) = control_command(&text) {
        return run_control(
            app,
            &input.session_id,
            &head.active_role,
            &text,
            input.user_message_id.as_deref(),
            cmd,
        )
        .await;
    }
    // Reject an unknown @mention up front, before it can sit in the queue.
    sessions::route(
        &state,
        &input.session_id,
        head.project_id.as_deref(),
        &head.active_role,
        &text,
    )?;

    // Older queued input goes first: between one turn finishing and the drain claiming the
    // next batch the session is briefly idle, and a fresh submit must not overtake the queue.
    let queue_waiting =
        !crate::db::inbox::list_inbox_internal(&state, &input.session_id, false)?.is_empty();
    let reservation = if queue_waiting {
        None
    } else {
        turns::reserve(&input.session_id, input.assistant_message_id.as_deref())
    };
    let Some(reservation) = reservation else {
        let item = queue::enqueue(
            app,
            &state,
            &input.session_id,
            &head.active_role,
            &text,
            &input.attachments,
        )?;
        // The running turn may have finished (and found an empty queue) in between.
        kick(app, &input.session_id);
        return Ok(SubmitResult::Queued { item });
    };

    spawn_session_loop(
        app.clone(),
        input.session_id.clone(),
        PendingTurn {
            reservation,
            text,
            attachments: input.attachments,
            user_message_id: input.user_message_id,
            runtime_kind: input.runtime_kind,
            batch: None,
        },
    );
    Ok(SubmitResult::Started {})
}

/// Start draining the queue if the session is idle and has queued input.
pub(crate) fn kick(app: &AppHandle, session_id: &str) {
    if let Some(next) = next_from_queue(app, session_id) {
        spawn_session_loop(app.clone(), session_id.to_string(), next);
    }
}

fn next_from_queue(app: &AppHandle, session_id: &str) -> Option<PendingTurn> {
    let state = app.state::<AppState>();
    // Queued input waits while the session is closed; reopening it resumes the drain.
    load_head(&state, session_id).ok()?;
    let reservation = turns::reserve(session_id, None)?;
    let batch = queue::claim_next(&state, session_id).ok().flatten()?;
    let text = match batch.role_name.as_deref() {
        // Queued for a persona other than the current one: address it explicitly, as the
        // desktop did when it drained the queue itself.
        Some(role)
            if !batch.text.starts_with('@')
                && load_head(&state, session_id)
                    .map(|h| h.active_role != role)
                    .unwrap_or(false) =>
        {
            format!("@{role} {}", batch.text)
        }
        _ => batch.text.clone(),
    };
    Some(PendingTurn {
        reservation,
        text,
        attachments: batch.attachments.clone(),
        user_message_id: None,
        runtime_kind: None,
        batch: Some(batch),
    })
}

fn spawn_session_loop(app: AppHandle, session_id: String, first: PendingTurn) {
    tauri::async_runtime::spawn(async move {
        let mut next = Some(first);
        while let Some(pending) = next.take() {
            // A turn that could not start put its batch back; draining again would claim the
            // same batch and fail the same way, spinning without ever yielding.
            if !run_one(&app, &session_id, pending).await {
                break;
            }
            next = next_from_queue(&app, &session_id);
        }
    });
}

/// Returns whether a turn actually began (it may still have failed afterwards).
async fn run_one(app: &AppHandle, session_id: &str, pending: PendingTurn) -> bool {
    let PendingTurn {
        reservation,
        text,
        attachments,
        user_message_id,
        runtime_kind,
        batch,
    } = pending;
    let state = app.state::<AppState>();
    // A turn that cannot begin still has to end visibly: the sender's window is showing
    // "sending" for a submit that was accepted, and only `turn/finished` clears that.
    let restore_batch = |app: &AppHandle, error: &str| match &batch {
        Some(batch) => queue::restore(app, &state, session_id, batch),
        None => super::events::emit(
            super::events::TURN_FINISHED,
            json!({
                "appSessionId": session_id,
                "turnId": "",
                "ok": false,
                "cancelled": false,
                "isCommand": false,
                "error": error,
                "messages": [],
            }),
        ),
    };
    let head = match load_head(&state, session_id) {
        Ok(head) => head,
        Err(e) => {
            eprintln!("[engine] cannot start turn in {session_id}: {e}");
            restore_batch(app, &e);
            return false;
        }
    };

    let mut patch = Map::new();
    let mut active_role = head.active_role.clone();
    let route = sessions::route(
        &state,
        session_id,
        head.project_id.as_deref(),
        &active_role,
        &text,
    );
    let mut runtime = runtime_kind.filter(|r| !r.trim().is_empty());
    if let Ok(route) = &route {
        if let Some((role, role_runtime, profile)) = &route.activate {
            active_role = role.clone();
            if role_runtime.is_some() {
                runtime = role_runtime.clone();
            }
            let _ = crate::db::app_session::set_app_session_route(
                &state,
                session_id,
                role,
                role_runtime.as_deref(),
                profile.as_deref(),
            );
            patch.insert("activeRole".into(), json!(role));
            if let Some(rt) = role_runtime {
                patch.insert("runtimeKind".into(), json!(rt));
                patch.insert("runtimeProfileId".into(), json!(profile));
            }
        }
    }
    if runtime.is_none() {
        runtime = head.runtime_kind.clone().filter(|r| !r.trim().is_empty());
    }
    if runtime.is_none() {
        // A session that never picked an engine gets the preferred one, durably, so every
        // client shows the same engine afterwards.
        if let Some((key, profile)) =
            sessions::preferred_runtime(&state, head.project_id.as_deref(), &active_role).await
        {
            let _ = crate::db::app_session::set_app_session_route(
                &state,
                session_id,
                &active_role,
                Some(&key),
                Some(&profile),
            );
            patch.insert("runtimeKind".into(), json!(key));
            patch.insert("runtimeProfileId".into(), json!(profile));
            runtime = Some(key);
        }
    }
    if let Some(title) = sessions::auto_title(&state, session_id, &head.title, &text) {
        patch.insert("title".into(), json!(title));
    }

    let label = route
        .as_ref()
        .map(|r| r.label.clone())
        .unwrap_or_else(|_| active_role.clone());
    let turn = match turns::begin(
        app,
        &state,
        reservation,
        TurnStart {
            role_label: &label,
            user_text: &text,
            images: &attachments,
            user_message_id: user_message_id.as_deref(),
            session_patch: Value::Object(patch),
        },
    ) {
        Ok(turn) => turn,
        Err(e) => {
            eprintln!("[engine] could not record turn in {session_id}: {e}");
            restore_batch(app, &e);
            return false;
        }
    };
    if let Some(batch) = &batch {
        queue::consume(app, &state, session_id, batch);
    }

    let route = match route {
        Ok(route) => route,
        Err(e) => {
            turn.finish(TurnOutcome::Failed, Some(&e));
            return true;
        }
    };
    let input = crate::types::AssistantChatInput {
        input: text,
        runtime_kind: runtime,
        app_session_id: Some(session_id.to_string()),
        attachments,
        context: crate::types::ChatContextOptions::default(),
    };
    match crate::chat::run_turn(
        app.clone(),
        app.state::<AppState>(),
        input,
        route.targets,
        &active_role,
    )
    .await
    {
        Ok(res) => {
            let outcome = if res.command_result.is_some() {
                TurnOutcome::Command { reply: &res.reply }
            } else {
                TurnOutcome::Reply {
                    ok: res.ok,
                    reply: &res.reply,
                    role_replies: &res.role_replies,
                }
            };
            turn.finish(outcome, (!res.ok).then_some(res.reply.as_str()));
        }
        Err(e) => {
            turn.finish(TurnOutcome::Failed, Some(&e));
        }
    }
    true
}

/// Personas with a live provider turn in this session — every one of them for a multi-agent
/// turn, not just the active persona.
fn running_roles(state: &AppState, session_id: &str, active_role: &str) -> Vec<String> {
    let mut roles: Vec<String> = crate::db::lifecycle::list_internal(state, session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| matches!(entry.state.as_str(), "running" | "prewarming"))
        .map(|entry| entry.role_name)
        .collect();
    if !roles.iter().any(|r| r == active_role) {
        roles.push(active_role.to_string());
    }
    roles.sort_unstable();
    roles.dedup();
    roles
}

/// Fire-and-forget cancel for callers without an async context (closing a session).
pub(crate) fn cancel_in_background(session_id: &str) {
    let Some(app) = super::events::app() else {
        return;
    };
    let session_id = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        let _ = cancel(&app, &session_id).await;
    });
}

/// Cancel the session's running turn. The turn still ends through `turn/finished`, with
/// whatever it had produced kept; queued input then drains as usual.
pub(crate) async fn cancel(app: &AppHandle, session_id: &str) -> Result<bool, String> {
    if !turns::mark_cancel_requested(session_id) {
        return Ok(false);
    }
    let state = app.state::<AppState>();
    let head = load_head(&state, session_id)?;
    let mut first_error = None;
    for role in running_roles(&state, session_id, &head.active_role) {
        if let Err(e) = crate::commands::runtime_cmd::cancel_acp_session(
            app.state::<AppState>(),
            role,
            session_id.to_string(),
        )
        .await
        {
            first_error.get_or_insert(e);
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(true),
    }
}

async fn run_control(
    app: &AppHandle,
    session_id: &str,
    active_role: &str,
    text: &str,
    user_message_id: Option<&str>,
    cmd: &str,
) -> Result<SubmitResult, String> {
    let state = app.state::<AppState>();
    let id = user_message_id
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let user = json!({
        "id": id,
        "roleName": "user",
        "text": text,
        "at": crate::now_ms(),
    });
    crate::db::app_session::save_app_message_internal(
        &state,
        session_id,
        user.get("id").and_then(Value::as_str),
        "user",
        text,
        None,
        None,
    )?;
    let _ = app.emit(
        MESSAGE_ADDED_EVENT,
        json!({ "appSessionId": session_id, "message": user }),
    );
    let message = if cmd == "cancel" {
        if cancel(app, session_id).await? {
            format!("cancelled {active_role}")
        } else {
            "nothing is running".to_string()
        }
    } else {
        crate::commands::runtime_cmd::set_acp_mode(
            app.state::<AppState>(),
            active_role.to_string(),
            cmd.to_string(),
            session_id.to_string(),
        )
        .await?;
        format!("{active_role} mode → {cmd}")
    };
    Ok(SubmitResult::Control { message })
}

/// "Send now" for queued input: steer it into the running turn when the engine supports
/// that, otherwise stop the turn so the queue drains immediately.
pub(crate) async fn send_queued_now(app: &AppHandle, session_id: &str) -> Result<(), String> {
    if !turns::is_busy(session_id) {
        kick(app, session_id);
        return Ok(());
    }
    let state = app.state::<AppState>();
    let head = load_head(&state, session_id)?;
    let steerable = match &head.runtime_kind {
        Some(rt) => sessions::supports_steer(rt).await,
        None => false,
    };
    if steerable {
        let items = crate::db::inbox::list_inbox_internal(&state, session_id, false)?;
        let same_role = queue::select_batch(&items).filter(|b| {
            b.attachments.is_empty() && b.role_name.as_deref().is_none_or(|r| r == head.active_role)
        });
        if same_role.is_some() {
            if let Some(batch) = queue::claim_next(&state, session_id)? {
                let steered = crate::commands::runtime_cmd::steer_acp_session(
                    app.state::<AppState>(),
                    head.active_role.clone(),
                    session_id.to_string(),
                    batch.text.clone(),
                    Vec::new(),
                )
                .await;
                match steered {
                    Ok(()) => {
                        let user = json!({
                            "id": uuid::Uuid::new_v4().to_string(),
                            "roleName": "user",
                            "text": batch.text,
                            "at": crate::now_ms(),
                        });
                        crate::db::app_session::save_app_message_internal(
                            &state,
                            session_id,
                            user.get("id").and_then(Value::as_str),
                            "user",
                            &batch.text,
                            None,
                            None,
                        )?;
                        queue::consume(app, &state, session_id, &batch);
                        let _ = app.emit(
                            MESSAGE_ADDED_EVENT,
                            json!({ "appSessionId": session_id, "message": user }),
                        );
                        return Ok(());
                    }
                    // The turn can finish between the busy check and the steer; fall back to
                    // stop-and-drain, which is always safe.
                    Err(_) => queue::restore(app, &state, session_id, &batch),
                }
            }
        }
    }
    cancel(app, session_id).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Tauri commands (desktop adapter)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitInputArgs {
    session_id: String,
    text: String,
    #[serde(default)]
    attachments: Vec<ImageAttachment>,
    user_message_id: Option<String>,
    assistant_message_id: Option<String>,
    runtime_kind: Option<String>,
}

#[tauri::command]
pub(crate) async fn submit_input(
    app: AppHandle,
    input: SubmitInputArgs,
) -> Result<SubmitResult, String> {
    submit(
        &app,
        SubmitInput {
            session_id: input.session_id,
            text: input.text,
            attachments: input.attachments,
            user_message_id: input.user_message_id,
            assistant_message_id: input.assistant_message_id,
            runtime_kind: input.runtime_kind,
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn cancel_turn(app: AppHandle, session_id: String) -> Result<bool, String> {
    cancel(&app, &session_id).await
}

#[tauri::command]
pub(crate) async fn send_queued_input_now(
    app: AppHandle,
    session_id: String,
) -> Result<(), String> {
    send_queued_now(&app, &session_id).await
}

#[tauri::command]
pub(crate) fn remove_queued_input(
    app: AppHandle,
    session_id: String,
    id: String,
) -> Result<(), String> {
    queue::remove(&app, &app.state::<AppState>(), &session_id, &id)
}

#[tauri::command]
pub(crate) fn session_busy(session_id: String) -> bool {
    turns::is_busy(&session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_commands_match_whole_words_only() {
        assert_eq!(control_command("/plan"), Some("plan"));
        assert_eq!(control_command("/cancel now"), Some("cancel"));
        assert_eq!(control_command("/planner"), None);
        assert_eq!(control_command("/app_role x"), None);
        assert_eq!(control_command("plan"), None);
    }
}
