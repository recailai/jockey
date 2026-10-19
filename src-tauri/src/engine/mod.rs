//! The engine: conversation state every client (desktop UI, web companion, Jockey MCP)
//! shares. Adapters call into it; views only render what it records and announces.
//! See `docs/engine-service-layer.md`.

pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod queue;
pub(crate) mod service;
pub(crate) mod sessions;
pub(crate) mod transcript;
pub(crate) mod turns;

/// Wire the engine into the app: the event hub and the transcript taps. Called once at setup.
pub(crate) fn install(app: &tauri::AppHandle) {
    events::install(app.clone());
    turns::install(app);
}
