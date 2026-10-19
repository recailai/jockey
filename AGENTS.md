# Repository Guidelines

JockeyUI is a **Tauri 2 desktop app** — a multi-agent orchestrator ("Conductor") that coordinates AI CLI agents (Claude Code, Codex CLI, Pi CLI, Antigravity CLI). **ACP (Agent Client Protocol)** is the product-level event/permission envelope shown to the UI; not every runtime speaks ACP on the wire. Claude Code is reached through a headless `claude-native` runner or an ACP bridge, and Codex, Pi, and Antigravity use their native protocols (see [ACP Transport](#acp-transport-src-tauriacp)).

Further documentation is indexed in [docs/README.md](docs/README.md).

## Build, Test, and Development Commands

```bash
# Frontend & Tauri Development
pnpm dev          # Start the Vite dev server only (port 1420); use `pnpm tauri dev` for the app
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
  - `components/` — `ui/` (design-system primitives), `composer/`, `management/` (settings tabs), `sidebar/`, plus chat, git, files, and terminal panels.
  - `hooks/` — Session and agent state (`useAgentData`, `useAgentContext`, etc.).
  - `lib/` — `tauriApi.ts` (typed `invoke` wrappers), `engineEvents.ts` (engine event listeners), `acpEventBridge.ts` (live ACP stream), `commandUi/` (composer command registry), `browserMock.ts` (runs the UI without Tauri).
- `src-tauri/src/`: Rust backend.
  - `lib.rs` — Registers Tauri commands (`generate_handler!`), state, and startup recovery.
  - `engine/` — Engine service layer: turn lifecycle, queue, transcript recording, events, and engine commands.
  - `acp/` — Runtime adapters and transports (see below).
  - `chat.rs`, `chat/` — Chat core: prompt building, context bundle, and session runtime between the engine and `acp`.
  - `db/` — SQLite persistence (pooled, migrations gated on `user_version`).
  - `commands/` — Tauri `#[command]` handlers (runtime control, git, completion, ...).
  - `web_gateway/` — axum HTTP + SSE gateway for the phone companion (`companion.html`); token auth, default port 8420.
  - `jockey_mcp/` — Built-in MCP bridge exposing `get_session_context`, `list_roles`, `list_skills`, `get_skill`, `invoke_role`.
  - `importer/` — Claude/Codex/Pi transcript import.
  - `git/` — Git operations behind the `git_*_cmd` commands.
  - `runtime_kind.rs`, `runtime_profile.rs` — Canonical runtime keys and custom runtime profiles.
- `docs/`: Architecture notes, reviews, roadmap, and reference material (index in `docs/README.md`).
- `public/`: Static assets packaged by Vite.
- Generated output: `dist/` (frontend build) and `src-tauri/target/` (Rust build artifacts); do not commit these.

### Backend Internals (`src-tauri/src/lib.rs`)

`AppState` is `{ db: DbPool, role_cache: Arc<DashMap<String, Arc<Role>>> }` (`types.rs`). SQLite is pooled (`db/pool.rs`, `busy_timeout` 5000 ms) and migrations are gated on `user_version` (`db/mod.rs`). Domain objects — `Project`, `AppSession`, `AppMessage`, `Role`, session context entries, and the legacy `Workflow` — are SQLite-backed and serialized with `#[serde(rename_all = "camelCase")]`.

Key command groups exposed to the frontend via `invoke()` (representative names; the full list is the `generate_handler!` call in `lib.rs`):
- **Engine** (`engine/service.rs`): `submit_input`, `cancel_turn`, `send_queued_input_now`, `remove_queued_input`
- **Projects and sessions** (`engine/commands.rs`): `create_app_session`, `update_app_session`, `delete_app_session`, `reopen_app_session`, `get_app_sessions`, `create_project_cmd`, `delete_project_cmd`, `import_project_sessions_cmd`
- **Runtime control** (`commands/runtime_cmd.rs`): `cancel_acp_session`, `steer_acp_session`, `set_acp_mode`, `set_acp_config_option`, `respond_permission`
- **Roles and detection**: `upsert_role_cmd`, `list_roles`, `detect_assistants`
- **Session context**: `list_session_context_entries_cmd`, `set_session_context_entry_cmd`, `delete_session_context_entry_cmd`
- **Git**: `git_status_cmd`, `git_diff_cmd`, `git_commit_cmd`, `git_push_cmd`, `git_create_pr_cmd`, ...
- **Web gateway**: `get_gateway_status_cmd`, `start_gateway_cmd`, `stop_gateway_cmd`, `regenerate_gateway_token_cmd`
- **UI helpers**: `complete_mentions`

Chat input goes through the engine (`engine::service::submit`): `@role` mentions route the turn (`engine::sessions::route`), `/plan` `/act` `/auto` `/cancel` are engine control commands (`control_command` in `engine/service.rs`), and every other `/command` is passed to the agent CLI. Composer commands such as `/model` or `/usage` are desktop UI actions (`src/lib/commandUi/registry.ts`).

Workflows are legacy and minimal: `start_workflow` runs a workflow's steps sequentially through `acp::execute_runtime`, emitting `session/update` and `workflow/state_changed` events. Only the old "Automations" tab manages workflows; no UI calls `start_workflow`.

### ACP Transport (`src-tauri/src/acp/`)

- A dedicated OS thread runs a `current_thread` Tokio runtime + `LocalSet` so the adapter layer can keep provider sessions, UI callbacks, and terminal ownership on one thread. External callers send `WorkerMsg::Execute` or `WorkerMsg::Prewarm` via an unbounded channel.
- ACP 2.0 stable-v1 schema types are paired with Jockey's small JSONL JSON-RPC transport boundary in `acp/transport.rs`; this keeps request correlation, backpressure, stderr tails, cancellation, and process shutdown uniform across every adapter.
- Provider process slots are scoped by `app_session_id:runtime_key:role_name`, and are evicted on cwd/launch-contract changes, errors, cancellation, or idle timeout.
- Canonical runtime keys live in `runtime_kind.rs`: `claude-native`, `claude-code`, `codex-cli`, `pi-cli`, `antigravity-cli`. Legacy spellings (`claude`, `codex`, `pi`, `agy`, `gemini`, ...) are normalised. Custom ACP runtime profiles are stored via `db/profiles.rs` and `runtime_profile.rs`. A `mock` runtime is available for offline development.
- Supported runtimes:
  - `claude-native` → headless `claude -p` stream-json transport; permission requests come back through a bridge (`--__jockey-permission-bridge`, handled in `main.rs`).
  - `claude-code` → ACP bridge `@agentclientprotocol/claude-agent-acp` (uses a pinned managed install in production; `pnpm dlx` / `npx -y` are only allowed in debug builds or when `JOCKEY_ALLOW_PACKAGE_RUNNER=1` is explicitly set for diagnostics).
  - `codex-cli` → native `codex app-server` JSON-RPC transport.
  - `pi-cli` → Pi's native `--mode rpc` JSONL transport.
  - `antigravity-cli` → Antigravity's headless `stream-json` transport with a one-shot fallback for older builds.
- Native CLIs are resolved from the user's interactive shell PATH so GUI launches see the same installations as a terminal.

### Data Flow

```text
Desktop UI / web companion / Jockey MCP
  → invoke("submit_input") (or the gateway / MCP equivalent)
    → engine::service::submit
      → turns::reserve (atomic per-session busy flag)
      → per-session loop (queue = `session_inbox_messages` table)
        → chat core (chat.rs, chat/session_runtime.rs)
          → acp::execute_runtime → WorkerMsg → ACP worker thread
            → spawns CLI subprocess (ACP / native JSON-RPC / stream-json)
  live preview: "acp/delta", "acp/stream" → UI
  engine taps the same stream into TurnRecorder (engine/transcript.rs, checkpoint every 1.5 s) and persists
  engine events (engine/events.rs): turn/started, turn/finished, turn/progress, queue/changed,
    message/added, session/changed, project/changed
  engine::watch follows the shown session: late joiners pull the live turn on turn/progress;
    a provider transcript growing outside Jockey is imported (docs/architecture/session-sync.md)
    → Tauri listeners (src/lib/engineEvents.ts) and SSE (web gateway)
```

Rule: the engine owns every durable write; views are thin and only render engine state and events.

### Key Design Patterns

- **Engine-owned persistence and a single event emitter**: all transcript, queue, and session writes go through `engine/`; the UI and web companion never write messages themselves.
- **Per-session serial, cross-session parallel turns**: `turns::reserve` rejects a second concurrent turn on a session; new input is queued in the inbox and drained in order.
- **Interrupt-based cancel**: cancelling interrupts the turn and keeps the provider process warm (Claude interrupt control request, Codex `turn/interrupt`, Pi abort). The process is killed only if the same turn is still running after 3 s.
- **Built-in Jockey MCP bridge**: injected into agent sessions as a normal MCP server (`jockey_mcp/`), not a separate protocol path.
- **SSE focus filter**: the web gateway sends high-frequency stream frames (`acp/delta`, `acp/stream`) only to clients focused on that session (`/api/events?focus=<sid>`); lifecycle events and permissions go to every client, and a lagging client gets `resync`.

## Coding Style & Naming Conventions

- **TypeScript / SolidJS**: 2-space indentation, semicolons, `camelCase` for variables and functions, `PascalCase` for components and type names.
- **Rust**: Default `rustfmt` style (4-space indentation), `snake_case` for functions/variables, `CamelCase` for structs/enums.
- **Serialization**: Keep Tauri command payloads and frontend types in `camelCase` to match serde convention (`#[serde(rename_all = "camelCase")]`).
- **Modularity**: Prefer small, focused modules; place protocol/backend logic in `src-tauri/src/` and UI behavior in `src/`.

### UI Design System (Catalog-First)

- **Do NOT craft ad-hoc buttons, badges, inputs, or dialogs.** Always consume primitives from `src/components/ui/` (`Badge`, `Pill`, `StateDot`, `Button`, `FormField`, `Checkbox`, `Alert`, `Dialog`, `MasterDetailView`, `AsyncContent`).
- **Strict semantic tokens**: Do NOT use hardcoded colors (e.g. `text-blue-500`, `bg-amber-500/15`). Rely on `--ui-*` tokens or semantic tone properties (`neutral`, `info`, `success`, `warning`, `danger`).
- Refer to [docs/architecture/ui-design-and-components.md](docs/architecture/ui-design-and-components.md) for detailed UI component specifications.

### Native Protocol Standards

- **PiRpc**: Fire-and-forget UI updates (`setStatus`, `notify`, `setWidget`) must NOT be answered with synthetic JSON-RPC results.
- Interactive UI requests in Pi must be answered with `{"type": "extension_ui_response", ...}`.
- In `extract_pi_session_id`, always prioritize clean `sessionId` over `sessionFile` paths.

## Testing Guidelines

- Primary automated tests are Rust-side via `cargo test`.
- Add unit tests next to Rust modules when introducing non-trivial logic.
- Frontend test framework is not configured yet; include manual verification steps in PRs (e.g., project creation, session import, role binding, web companion flow).

## Commit & Pull Request Guidelines

- Follow Conventional Commits: `feat: ...`, `docs: ...`, `chore: ...`.
- Keep commits scoped to one change theme; use imperative, specific summaries.
- PRs should include:
  - Concise problem/solution description,
  - Linked issue (if available),
  - Testing evidence (`cargo test`, `pnpm build`, manual UI checks),
  - Screenshots or short recordings for UI changes.

## Current State & Roadmap

Live adapters for Claude Code (native and ACP bridge), Codex CLI, Pi CLI, and Antigravity CLI are implemented under `src-tauri/src/acp/`; ACP remains the product-level event/permission envelope, while Codex/Pi/agy use their native or headless protocols where that is more reliable than a bridge. Model catalogs are discovered at runtime (`model/list` for Codex and `get_available_models` for Pi) and can be explicitly refreshed from the role editor. Mock transport remains available for offline development.

Session recovery after restart: `engine::recover_after_restart` (run at startup in `lib.rs`) releases claimed inbox items and resets stale agent lifecycle rows; provider sessions are resumed from persisted handles (see `docs/architecture/runtime-adapters.md`).

Pending work is tracked in [docs/roadmap/pending.md](docs/roadmap/pending.md); the latest audit is [docs/reviews/review-2026-10.md](docs/reviews/review-2026-10.md).
