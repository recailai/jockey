//! Fork a session: the provider branches its own conversation (ACP `session/fork`) and the
//! engine mirrors that as a new session holding a copy of the transcript. Nothing is written
//! unless the provider fork succeeded, so a failure never leaves a replacement conversation.

use super::events::{self, SessionChange};
use crate::types::{AppSession, AppState};

const FORK_PREFIX: &str = "[Fork]_";

/// Titles are single tokens (whitespace becomes `_`), so the marker is `[Fork]_`. Forking a
/// fork keeps one marker instead of stacking them.
pub(crate) fn fork_title(title: &str) -> String {
    let base = title.strip_prefix(FORK_PREFIX).unwrap_or(title);
    format!("{FORK_PREFIX}{base}")
}

pub(crate) async fn fork_session(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
) -> Result<AppSession, String> {
    let result = fork_reserved(state, id).await;
    // Input submitted while the fork held the session was queued; nothing else will drain it.
    super::service::kick(app, id);
    result
}

async fn fork_reserved(state: &AppState, id: &str) -> Result<AppSession, String> {
    // Held until the fork is stored, so no turn can start between the provider fork and the
    // transcript copy (the copy would otherwise miss or half-include that turn).
    let _reservation = super::turns::reserve(id, None)
        .ok_or_else(|| "Wait for the running turn to finish before forking".to_string())?;
    let head = crate::db::app_session::load_session_head(state, id)?
        .ok_or_else(|| "session not found".to_string())?;
    let role = head.active_role.clone();
    let runtime = super::sessions::resolve_role_runtime(state, id, &role, None, None)
        .ok_or_else(|| format!("runtime not found for role: {role}"))?;
    let runtime = crate::runtime_profile::runtime_key_static(&runtime)
        .map(str::to_string)
        .unwrap_or(runtime);
    if crate::db::app_session_role::load_app_session_role_cli_id(state, id, &runtime, &role)
        .is_none()
    {
        return Err("Nothing to fork yet: this session has no provider conversation".to_string());
    }

    // Connect (or resume) the provider session so `session/fork` has a live slot to run on.
    // The worker is the authority on whether the adapter negotiated fork; its error is
    // passed through rather than re-checked here.
    let cwd = crate::db::app_session::get_app_session_cwd(state, id)
        .unwrap_or_else(crate::default_chat_cwd);
    crate::acp::prewarm_role(&runtime, &role, &cwd, Some((state, id))).await;

    let provider_id = crate::acp::fork_session(&runtime, &role, id)
        .await
        .map_err(|e| match e.as_str() {
            "no active session" => "Could not connect the agent to fork this session".to_string(),
            _ => e,
        })?;
    if provider_id.trim().is_empty() {
        return Err("The agent returned no session id for the fork".to_string());
    }

    let mut forked = crate::db::app_session::fork_app_session_internal(
        state,
        id,
        &fork_title(&head.title),
        Some((&runtime, &role, &provider_id)),
    )?;
    forked.messages = crate::db::app_session::get_app_sessions_by_ids(state, &[forked.id.clone()])?
        .into_iter()
        .next()
        .map(|s| s.messages)
        .unwrap_or_default();
    events::session_changed(state, &forked.id, SessionChange::Created);
    Ok(forked)
}

#[cfg(test)]
mod tests {
    use super::fork_title;

    #[test]
    fn fork_title_marks_once() {
        assert_eq!(fork_title("Fix_login"), "[Fork]_Fix_login");
        assert_eq!(fork_title("[Fork]_Fix_login"), "[Fork]_Fix_login");
    }
}
