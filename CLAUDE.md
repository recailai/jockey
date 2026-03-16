# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development
pnpm dev          # Start Vite dev server (port 1420) + Tauri hot reload
pnpm build        # Build production frontend
pnpm tauri dev    # Start full Tauri desktop app (frontend + Rust backend)
pnpm tauri build  # Build distributable app bundle

# Rust (from project root — use --manifest-path for CI or scripts)
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

## Architecture

UnionAI is a **Tauri 2 desktop app** — a multi-agent orchestrator ("Conductor") that coordinates AI CLI agents (Claude Code, Gemini CLI, Codex CLI) via the **ACP (Agent Client Protocol)** JSON-RPC-over-stdio standard.

### File Layout

- `src/App.tsx` — Entire SolidJS frontend. Single-file chat UI with `/commands`, `@mentions`, streaming display, role creation form.
- `src-tauri/src/lib.rs` — Entire Rust backend (~3300 lines). All Tauri `#[command]` handlers, SQLite schema, workflow engine, chat command parser, assistant runtime detection.
- `src-tauri/src/acp.rs` — ACP transport layer. Manages a single-threaded Tokio worker for `!Send` ACP connections, connection pooling per `runtime_key:role_name` slot, streaming deltas via channels.

### Backend Internals (lib.rs)

`AppState` holds a `Mutex<Connection>` (SQLite) and a `DashMap<String,String>` for shared context. All domain objects — `Team`, `Role`, `Workflow`, `Session`, `SessionEvent`, `ContextEntry` — are SQLite-backed and serialized with `#[serde(rename_all = "camelCase")]`.

Key command groups exposed to the frontend via `invoke()`:
- **CRUD**: `upsert_role_cmd`, `list_roles`, `create_workflow`, `list_workflows`, `list_sessions`, `list_session_events`
- **Context**: `set_shared_context`, `get_shared_context`, `list_shared_context`
- **Execution**: `start_workflow`, `assistant_chat`, `detect_assistants`
- **UI helpers**: `complete_mentions`, `complete_cli`, `apply_chat_command`

`apply_chat_command` is the main entry point for the chat command system — it parses `/team`, `/role`, `/workflow`, `/run`, `/context`, `/assistant`, `/help` commands and dispatches to the appropriate handler.

`start_workflow` runs a workflow's steps sequentially, calling `acp::execute_runtime` for each role, emitting `session-update` and `workflow-state` Tauri events for streaming UI updates.

### ACP Transport (acp.rs)

A dedicated OS thread runs a `current_thread` Tokio runtime + `LocalSet` (required because `acp::ClientSideConnection` is `!Send`). External callers send `WorkerMsg::Execute` or `WorkerMsg::Prewarm` via an unbounded channel. Connections are pooled per `runtime_key:role_name` in a `DashMap<String, Arc<SlotHandle>>` and evicted on cwd change or error.

Supported runtimes resolved via `build_stdio_adapter`: `claude`/`claude-code` → `claude-agent-acp`, `gemini`/`gemini-cli` → `gemini --experimental-acp`, `codex`/`codex-cli` → `codex-acp`. Falls back to `pnpm dlx` / `npx -y` if native binary is not found.

### Data Flow

```
User types in App.tsx input
  → invoke("assistant_chat" | "apply_chat_command")
    → lib.rs command handler
      → SQLite for state mutations
      → acp::execute_runtime → WorkerMsg → ACP worker thread
        → spawns CLI subprocess, stdio JSON-RPC
        → streaming deltas emitted via Tauri events ("acp/delta", "session-update", "workflow-state")
  → Frontend listens via @tauri-apps/api/event, renders streaming chunks
```

### Design Patterns

- **MCP-over-ACP**: Tool/resource provisioning embedded within ACP channels, no separate MCP processes (see `docs/rfd_mcp_over_acp.mdx`).
- **Session forking**: Forked sessions for summaries/PR descriptions without polluting main history (`docs/rfd_session_fork.mdx`).
- **Proxy chains**: Middleware for intercepting/transforming agent messages (`docs/rfd_proxy_chains.mdx`).
- **W3C Trace Context**: `_meta` field propagation for distributed tracing (`docs/rfd_meta_propagation.mdx`).

## Conventions

- TypeScript/Solid: 2-space indent, semicolons, `camelCase` vars/functions, `PascalCase` components/types.
- Rust: `rustfmt` defaults, `snake_case` functions, `CamelCase` structs/enums.
- Tauri command payloads use `camelCase` via `#[serde(rename_all = "camelCase")]`.
- Commits follow Conventional Commits: `feat:`, `docs:`, `chore:`.
- No frontend test framework configured yet — Rust tests via `cargo test`.

## Current State (v0.1.0 MVP)

Live ACP adapters for Claude Code, Gemini CLI, and Codex CLI are implemented in `acp.rs`. Mock transport still available for offline development. Pending work tracked in `todo.md`: native macOS window polish, virtual list for long sessions, session recovery after restart.
