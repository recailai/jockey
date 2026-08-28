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

JockeyUI is a **Tauri 2 desktop app** — a multi-agent orchestrator ("Conductor") that coordinates AI CLI agents (Claude Code, Antigravity CLI, Codex CLI) via the **ACP (Agent Client Protocol)** JSON-RPC-over-stdio standard.

### File Layout

- `src/` — SolidJS frontend. `App.tsx` is the main view entry; `components/` holds chat, permission, and management tabs; `hooks/` owns session/agent state; `lib/` bridges ACP events and the Tauri API.
- `src-tauri/src/` — Rust backend. `lib.rs` registers Tauri commands and app state; `acp/` owns runtime adapters and transports; `db/` is the SQLite persistence layer; `commands/` holds `#[command]` handler impls.
- `src-tauri/src/acp/` — ACP-compatible transport layer. Keeps the legacy `!Send` ACP worker, native JSONL runners, headless stream runner, adapter resolution, and shared session lifecycle in focused modules.

### Backend Internals (lib.rs)

`AppState` holds a `Mutex<Connection>` (SQLite) and a `DashMap<String,String>` for shared context. All domain objects — `Team`, `Role`, `Workflow`, `Session`, `SessionEvent`, `ContextEntry` — are SQLite-backed and serialized with `#[serde(rename_all = "camelCase")]`.

Key command groups exposed to the frontend via `invoke()`:
- **CRUD**: `upsert_role_cmd`, `list_roles`, `create_workflow`, `list_workflows`, `list_sessions`, `list_session_events`
- **Context**: `set_shared_context`, `get_shared_context`, `list_shared_context`
- **Execution**: `start_workflow`, `assistant_chat`, `detect_assistants`
- **UI helpers**: `complete_mentions`, `complete_cli`, `apply_chat_command`

`apply_chat_command` is the main entry point for the chat command system — it parses `/team`, `/role`, `/workflow`, `/run`, `/context`, `/assistant`, `/help` commands and dispatches to the appropriate handler.

`start_workflow` runs a workflow's steps sequentially, calling `acp::execute_runtime` for each role, emitting `session-update` and `workflow-state` Tauri events for streaming UI updates.

### ACP Transport (`src-tauri/src/acp/`)

A dedicated OS thread runs a `current_thread` Tokio runtime + `LocalSet` so the adapter layer can keep provider sessions, UI callbacks, and terminal ownership on one thread. External callers send `WorkerMsg::Execute` or `WorkerMsg::Prewarm` via an unbounded channel. ACP 2.0 stable-v1 schema types are paired with Jockey's small JSONL JSON-RPC transport boundary in `acp/transport.rs`; this keeps request correlation, backpressure, stderr tails, cancellation, and process shutdown uniform across every adapter. Provider process slots are scoped by `app_session_id:runtime_key:role_name`, and are evicted on cwd/launch-contract changes, errors, cancellation, or idle timeout.

Supported runtimes resolved via `build_stdio_adapter`: `claude`/`claude-code` → `@agentclientprotocol/claude-agent-acp`, `codex`/`codex-cli` → the native `codex app-server` JSON-RPC transport, `pi`/`pi-cli` → Pi's native `--mode rpc` JSONL transport, and `agy`/`antigravity-cli` → Antigravity's headless `stream-json` transport with a one-shot fallback for older builds. Legacy `gemini`/`gemini-cli` values normalize to `antigravity-cli`. Claude's ACP bridge uses a pinned managed install in production; `pnpm dlx` / `npx -y` are only allowed in debug builds or when `JOCKEY_ALLOW_PACKAGE_RUNNER=1` is explicitly set for diagnostics. Native CLIs are resolved from the user's interactive shell PATH so GUI launches see the same installations as a terminal.

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

Live adapters for Claude Code, Codex CLI, Pi CLI, and Antigravity CLI are implemented under `src-tauri/src/acp/`; ACP remains the product-level event/permission envelope, while Codex/Pi/agy use their native or headless protocols where that is more reliable than a bridge. Model catalogs are discovered at runtime (`model/list` for Codex and `get_available_models` for Pi) and can be explicitly refreshed from the role editor. Mock transport remains available for offline development. Pending work tracked in `todo.md`: native macOS window polish, virtual list for long sessions, provider session recovery after app restart.
