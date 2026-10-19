//! Input submitted while a session is busy. Stored in the session inbox table (so it survives
//! a restart) and drained by the engine when the running turn finishes — not by whichever
//! window happened to queue it.

use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::types::{AppState, ImageAttachment, InboxMessage};

pub(crate) const QUEUE_CHANGED_EVENT: &str = "queue/changed";

/// Announce the session's current queue. Views replace their copy with this list.
pub(crate) fn announce(app: &AppHandle, state: &AppState, session_id: &str) {
    let items = crate::db::inbox::list_inbox_internal(state, session_id, false).unwrap_or_default();
    let _ = app.emit(
        QUEUE_CHANGED_EVENT,
        json!({ "appSessionId": session_id, "items": items }),
    );
}

pub(crate) fn enqueue(
    app: &AppHandle,
    state: &AppState,
    session_id: &str,
    role_name: &str,
    text: &str,
    attachments: &[ImageAttachment],
) -> Result<InboxMessage, String> {
    let item = crate::db::inbox::enqueue_inbox_internal(
        state,
        session_id,
        Some(role_name),
        "nextTurn",
        text,
        attachments,
    )?;
    announce(app, state, session_id);
    Ok(item)
}

pub(crate) fn remove(
    app: &AppHandle,
    state: &AppState,
    session_id: &str,
    id: &str,
) -> Result<(), String> {
    crate::db::inbox::remove_inbox_internal(state, id)?;
    announce(app, state, session_id);
    Ok(())
}

/// A claimed run of queued input, ready to become one turn.
pub(crate) struct Batch {
    pub(crate) ids: Vec<String>,
    pub(crate) role_name: Option<String>,
    pub(crate) text: String,
    pub(crate) attachments: Vec<ImageAttachment>,
}

/// Leading queued items addressed to the same persona, merged into one prompt — the batching
/// the desktop's `projectQueuedItemDequeue` did.
pub(crate) fn select_batch(items: &[InboxMessage]) -> Option<Batch> {
    let first = items.first()?;
    let role = first.role_name.clone();
    let run: Vec<&InboxMessage> = items
        .iter()
        .take_while(|item| item.role_name == role)
        .collect();
    let text = run
        .iter()
        .map(|item| item.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Batch {
        ids: run.iter().map(|item| item.id.clone()).collect(),
        role_name: role,
        text,
        attachments: run
            .iter()
            .flat_map(|item| item.attachments.clone())
            .collect(),
    })
}

/// Claim the next batch so no other drainer can take it. `None` when the queue is empty.
pub(crate) fn claim_next(state: &AppState, session_id: &str) -> Result<Option<Batch>, String> {
    let items = crate::db::inbox::list_inbox_internal(state, session_id, false)?;
    let Some(batch) = select_batch(&items) else {
        return Ok(None);
    };
    crate::db::inbox::claim_inbox_internal(state, &batch.ids)?;
    Ok(Some(batch))
}

/// The batch became a turn (or was steered in); drop it from the inbox for good.
pub(crate) fn consume(app: &AppHandle, state: &AppState, session_id: &str, batch: &Batch) {
    for id in &batch.ids {
        let _ = crate::db::inbox::remove_inbox_internal(state, id);
    }
    announce(app, state, session_id);
}

/// The batch could not be delivered; put it back at the front of the queue.
pub(crate) fn restore(app: &AppHandle, state: &AppState, session_id: &str, batch: &Batch) {
    let _ = crate::db::inbox::restore_inbox_internal(state, &batch.ids);
    announce(app, state, session_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, role: Option<&str>, text: &str) -> InboxMessage {
        InboxMessage {
            id: id.into(),
            app_session_id: "s".into(),
            role_name: role.map(str::to_string),
            delivery: "nextTurn".into(),
            text: text.into(),
            attachments: Vec::new(),
            created_at: 0,
        }
    }

    #[test]
    fn batch_merges_leading_items_for_one_role() {
        let items = vec![
            item("1", Some("Developer"), "first"),
            item("2", Some("Developer"), " second "),
            item("3", Some("Reviewer"), "third"),
            item("4", Some("Developer"), "fourth"),
        ];
        let batch = select_batch(&items).unwrap();
        assert_eq!(batch.ids, ["1", "2"]);
        assert_eq!(batch.text, "first\nsecond");
        assert_eq!(batch.role_name.as_deref(), Some("Developer"));
        assert!(select_batch(&[]).is_none());
    }
}
