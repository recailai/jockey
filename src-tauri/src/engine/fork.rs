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

pub(crate) async fn fork_session(state: &AppState, id: &str) -> Result<AppSession, String> {
    let head = crate::db::app_session::load_session_head(state, id)?
        .ok_or_else(|| "session not found".to_string())?;
    if super::turns::is_busy(id) {
        return Err("Wait for the running turn to finish before forking".to_string());
    }
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
    // Prewarm also reveals whether the adapter supports fork on a cold start.
    let cwd = crate::db::app_session::get_app_session_cwd(state, id)
        .unwrap_or_else(crate::default_chat_cwd);
    crate::acp::prewarm_role(&runtime, &role, &cwd, Some((state, id))).await;
    if crate::acp::runtime_fork(&runtime) != Some(true) {
        return Err("This ACP adapter has not negotiated session fork.".to_string());
    }

    let provider_id = crate::acp::fork_session(&runtime, &role, id).await?;
    if provider_id.trim().is_empty() {
        return Err("The agent returned no session id for the fork".to_string());
    }

    let mut forked =
        crate::db::app_session::fork_app_session_internal(state, id, &fork_title(&head.title))?;
    crate::db::app_session_role::save_app_session_role_cli_id(
        state,
        &forked.id,
        &runtime,
        &role,
        &provider_id,
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
