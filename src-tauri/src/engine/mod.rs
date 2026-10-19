//! The engine: conversation state every client (desktop UI, web companion, Jockey MCP)
//! shares. Adapters call into it; views only render what it records and announces.
//! See `docs/engine-service-layer.md`.

pub(crate) mod transcript;
pub(crate) mod turns;
