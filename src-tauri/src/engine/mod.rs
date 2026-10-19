//! The engine: conversation state every client (desktop UI, web companion, Jockey MCP)
//! shares. Adapters call into it; views only render what it records and announces.
//! See `docs/architecture/engine-service-layer.md`.

pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod queue;
pub(crate) mod service;
pub(crate) mod sessions;
pub(crate) mod transcript;
pub(crate) mod turns;
pub(crate) mod watch;

/// Wire the engine into the app: the event hub and the transcript taps. Called once at setup.
pub(crate) fn install(app: &tauri::AppHandle) {
    events::install(app.clone());
    turns::install(app);
}

/// Undo state a crash or force-quit left behind: inbox items claimed by a turn that never
/// began, and agent lifecycle rows still marked live. Runs once at startup, before any drain.
pub(crate) fn recover_after_restart(state: &crate::types::AppState) -> Result<(), String> {
    crate::db::with_db(state, |conn| {
        conn.execute(
            "UPDATE session_inbox_messages SET claimed_at = NULL WHERE claimed_at IS NOT NULL",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE agent_lifecycle SET state = 'stopped'
             WHERE state IN ('running', 'prewarming', 'stopping')",
            [],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}
