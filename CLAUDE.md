# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development
pnpm dev          # Start Vite dev server (port 1420) + Tauri hot reload
pnpm build        # Build production frontend
pnpm tauri dev    # Start full Tauri desktop app (frontend + Rust backend)
pnpm tauri build  # Build distributable app bundle

# Rust (run from src-tauri/ or project root)
cargo check       # Fast type-check without compiling
cargo clippy      # Lint Rust code
cargo fmt         # Format Rust code
cargo test        # Run Rust tests
```

## Architecture

UnionAI is a **Tauri 2 desktop app** that acts as a multi-agent orchestrator — a "Conductor" that coordinates multiple AI CLI agents (Claude Code, Gemini CLI, Ollama, etc.) following the **ACP (Agent Client Protocol)** JSON-RPC-over-stdio standard.

### Core Components

**`src-tauri/src/lib.rs`** (~2000 lines) — The entire Rust backend. Contains:
- `Conductor` — Central orchestrator. Manages Teams, Roles, Sessions, and the Shared Brain. All Tauri `#[command]` handlers live here.
- `SharedBrain` — Two-tier memory: L1 hot cache (`DashMap`) for nanosecond reads, L2 cold (`SQLite` via `rusqlite`) for persistence/recovery.
- `Team` / `Role` — A Team is an isolated workspace unit; a Role is a named agent position (Architect, Developer, Reviewer) bound to a specific CLI runtime.
- `WorkflowEngine` — Sequential task orchestration with checkpoints, conditional routing, and approval gates.
- `SessionManager` — Tracks ACP session state machines (Thinking → Idle → Paused → Errored).

**`src-tauri/src/acp.rs`** — ACP protocol types and the `AcpTransport` trait. Defines the message envelope format for agent communication. Currently contains a `MockTransport` stub — real adapters for Claude Code / Gemini are on the TODO list.

**`src/App.tsx`** — The entire Solid.js frontend. A chat-style UI where users issue `/commands` that call Tauri `invoke()` → Rust backend. Commands: `/team`, `/role`, `/workflow`, `/run`, `/context`, `/assistant`.

### Data Flow

```
User types command in App.tsx
  → invoke("command_name", args) via @tauri-apps/api
    → Conductor handler in lib.rs
      → SharedBrain (DashMap/SQLite state)
      → AcpTransport → AI CLI process (stdio JSON-RPC)
        → Response propagated back through Tauri events
```

### Key Design Patterns

- **No separate MCP processes**: MCP-over-ACP embeds tool/resource provisioning within ACP channels (see `docs/rfd_mcp_over_acp.mdx`).
- **Session forking**: Create forked sessions for summaries/PR descriptions without polluting main conversation history (`docs/rfd_session_fork.mdx`).
- **Proxy chains**: Middleware-like extensibility for intercepting and transforming agent messages (`docs/rfd_proxy_chains.mdx`).
- **W3C Trace Context**: `_meta` field propagation for distributed tracing across agents (`docs/rfd_meta_propagation.mdx`).

### Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop shell | Tauri 2 |
| Frontend | Solid.js 1.9, TypeScript 5, TailwindCSS 4, Vite 7 |
| Backend runtime | Rust (stable), Tokio async |
| State (hot) | DashMap 6 |
| State (cold) | SQLite (bundled, WAL mode) |
| Agent protocol | ACP (JSON-RPC over stdio) |

### Current State (v0.1.0 MVP)

Real ACP process adapters for Claude/Gemini are **not yet implemented** — `MockTransport` is used. Session recovery after restart, native macOS UI polish, and virtual list optimization are pending (see `todo.md`).
