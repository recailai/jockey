# Repository Guidelines

JockeyUI is a **Tauri 2 desktop app** — a multi-agent orchestrator ("Conductor") that coordinates AI CLI agents (Claude Code, Antigravity CLI, Codex CLI, Pi CLI) via the **ACP (Agent Client Protocol)** JSON-RPC-over-stdio standard.

## Build, Test, and Development Commands

```bash
# Frontend & Tauri Development
pnpm dev          # Start Vite dev server (port 1420) + Tauri hot reload
pnpm build        # Build production frontend into dist/
pnpm tauri dev    # Start full Tauri desktop app (frontend + Rust backend)
pnpm tauri build  # Build distributable app bundle

# Rust Backend (from project root — use --manifest-path for CI or scripts)
cargo check --manifest-path src-tauri/Cargo.toml  # Fast Rust type check
cargo test --manifest-path src-tauri/Cargo.toml   # Run Rust tests
cargo clippy --manifest-path src-tauri/Cargo.toml # Rust lint checks
cargo fmt --manifest-path src-tauri/Cargo.toml    # Format Rust code
```

## Project Structure & Architecture

### File Layout

- `src/`: SolidJS + TypeScript UI.
  - `App.tsx` — Main view entry.
  - `index.tsx` — Bootstraps app.
  - `index.css` — Tailwind and CSS design tokens (`--ui-*`).
  - `components/` — Chat, permission, and management tabs.
  - `hooks/` — Session and agent state (`useAgentData`, `useAgentContext`, etc.).
  - `lib/` — Bridges ACP events and the Tauri API.
- `src-tauri/src/`: Rust backend for Tauri commands and orchestration logic.
  - `lib.rs` — Registers Tauri commands and application state.
  - `acp/` — ACP-compatible runtime adapters and transports.
  - `db/` — SQLite persistence layer.
  - `commands/` — Tauri `#[command]` handler implementations.
- `src-tauri/src/acp/`: Transport layer keeping provider sessions, native JSONL runners, headless stream runner, adapter resolution, and shared session lifecycle in focused modules.
- `docs/`: Architecture notes, UI specs, and RFC-style design docs (RFDs).
- `public/`: Static assets packaged by Vite.
- Generated output: `dist/` (frontend build) and `src-tauri/target/` (Rust build artifacts); do not commit these.

### Backend Internals (`src-tauri/src/lib.rs`)

`AppState` holds a `Mutex<Connection>` (SQLite) and a `DashMap<String, String>` for shared context. All domain objects — `Team`, `Role`, `Workflow`, `Session`, `SessionEvent`, `ContextEntry` — are SQLite-backed and serialized with `#[serde(rename_all = "camelCase")]`.

Key command groups exposed to the frontend via `invoke()`:
- **CRUD**: `upsert_role_cmd`, `list_roles`, `create_workflow`, `list_workflows`, `list_sessions`, `list_session_events`
- **Context**: `set_shared_context`, `get_shared_context`, `list_shared_context`
- **Execution**: `start_workflow`, `assistant_chat`, `detect_assistants`
- **UI helpers**: `complete_mentions`, `complete_cli`, `apply_chat_command`

`apply_chat_command` is the main entry point for the chat command system — it parses `/team`, `/role`, `/workflow`, `/run`, `/context`, `/assistant`, `/help` commands and dispatches to the appropriate handler.

`start_workflow` runs a workflow's steps sequentially, calling `acp::execute_runtime` for each role, emitting `session-update` and `workflow-state` Tauri events for streaming UI updates.

### ACP Transport (`src-tauri/src/acp/`)

- A dedicated OS thread runs a `current_thread` Tokio runtime + `LocalSet` so the adapter layer can keep provider sessions, UI callbacks, and terminal ownership on one thread. External callers send `WorkerMsg::Execute` or `WorkerMsg::Prewarm` via an unbounded channel.
- ACP 2.0 stable-v1 schema types are paired with Jockey's small JSONL JSON-RPC transport boundary in `acp/transport.rs`; this keeps request correlation, backpressure, stderr tails, cancellation, and process shutdown uniform across every adapter.
- Provider process slots are scoped by `app_session_id:runtime_key:role_name`, and are evicted on cwd/launch-contract changes, errors, cancellation, or idle timeout.
- Supported runtimes resolved via `build_stdio_adapter`:
  - `claude` / `claude-code` → `@agentclientprotocol/claude-agent-acp` (uses a pinned managed install in production; `pnpm dlx` / `npx -y` are only allowed in debug builds or when `JOCKEY_ALLOW_PACKAGE_RUNNER=1` is explicitly set for diagnostics).
  - `codex` / `codex-cli` → native `codex app-server` JSON-RPC transport.
  - `pi` / `pi-cli` → Pi's native `--mode rpc` JSONL transport.
  - `agy` / `antigravity-cli` → Antigravity's headless `stream-json` transport with a one-shot fallback for older builds (legacy `gemini`/`gemini-cli` normalize to `antigravity-cli`).
- Native CLIs are resolved from the user's interactive shell PATH so GUI launches see the same installations as a terminal.

### Data Flow

```text
User types in App.tsx input
  → invoke("assistant_chat" | "apply_chat_command")
    → lib.rs command handler
      → SQLite for state mutations
      → acp::execute_runtime → WorkerMsg → ACP worker thread
        → spawns CLI subprocess, stdio JSON-RPC
        → streaming deltas emitted via Tauri events ("acp/delta", "session-update", "workflow-state")
  → Frontend listens via @tauri-apps/api/event, renders streaming chunks
```

### Key Design Patterns

- **MCP-over-ACP**: Tool/resource provisioning embedded within ACP channels, no separate MCP processes (see `docs/rfd_mcp_over_acp.mdx`).
- **Session forking**: Forked sessions for summaries/PR descriptions without polluting main history (`docs/rfd_session_fork.mdx`).
- **Proxy chains**: Middleware for intercepting/transforming agent messages (`docs/rfd_proxy_chains.mdx`).
- **W3C Trace Context**: `_meta` field propagation for distributed tracing (`docs/rfd_meta_propagation.mdx`).

## Coding Style & Naming Conventions

- **TypeScript / SolidJS**: 2-space indentation, semicolons, `camelCase` for variables and functions, `PascalCase` for components and type names.
- **Rust**: Default `rustfmt` style (4-space indentation), `snake_case` for functions/variables, `CamelCase` for structs/enums.
- **Serialization**: Keep Tauri command payloads and frontend types in `camelCase` to match serde convention (`#[serde(rename_all = "camelCase")]`).
- **Modularity**: Prefer small, focused modules; place protocol/backend logic in `src-tauri/src/` and UI behavior in `src/`.

### UI Design System (Catalog-First)

- **Do NOT craft ad-hoc buttons, badges, inputs, or dialogs.** Always consume primitives from `src/components/ui/` (`Badge`, `Pill`, `StateDot`, `Button`, `FormField`, `Checkbox`, `Alert`, `Dialog`, `MasterDetailView`, `AsyncContent`).
- **Strict semantic tokens**: Do NOT use hardcoded colors (e.g. `text-blue-500`, `bg-amber-500/15`). Rely on `--ui-*` tokens or semantic tone properties (`neutral`, `info`, `success`, `warning`, `danger`).
- Refer to [docs/ui_design_and_component_specification.md](file:///Users/sexy/Documents/GitHub/jockey/docs/ui_design_and_component_specification.md) for detailed UI component specifications.

### Native Protocol Standards

- **PiRpc**: Fire-and-forget UI updates (`setStatus`, `notify`, `setWidget`) must NOT be answered with synthetic JSON-RPC results.
- Interactive UI requests in Pi must be answered with `{"type": "extension_ui_response", ...}`.
- In `extract_pi_session_id`, always prioritize clean `sessionId` over `sessionFile` paths.

## Testing Guidelines

- Primary automated tests are Rust-side via `cargo test`.
- Add unit tests next to Rust modules when introducing non-trivial logic.
- Frontend test framework is not configured yet; include manual verification steps in PRs (e.g., team creation, role binding, assistant chat flow).

## Commit & Pull Request Guidelines

- Follow Conventional Commits: `feat: ...`, `docs: ...`, `chore: ...`.
- Keep commits scoped to one change theme; use imperative, specific summaries.
- PRs should include:
  - Concise problem/solution description,
  - Linked issue (if available),
  - Testing evidence (`cargo test`, `pnpm build`, manual UI checks),
  - Screenshots or short recordings for UI changes.

## Current State & Roadmap (v0.1.0 MVP)

Live adapters for Claude Code, Codex CLI, Pi CLI, and Antigravity CLI are implemented under `src-tauri/src/acp/`; ACP remains the product-level event/permission envelope, while Codex/Pi/agy use their native or headless protocols where that is more reliable than a bridge. Model catalogs are discovered at runtime (`model/list` for Codex and `get_available_models` for Pi) and can be explicitly refreshed from the role editor. Mock transport remains available for offline development. Pending work tracked in `todo.md`: native macOS window polish, virtual list for long sessions, provider session recovery after app restart.
