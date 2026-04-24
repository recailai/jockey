# ACP Tasks

## Already landed

Verified complete against the current tree.

### Module layout

- `acp/{adapter, client, connection, error, metrics, mod, runtime_state}.rs`
  plus `session/{cold_start, execute, prewarm, mcp, session_cmds}.rs` and
  `worker/{mod, handlers, permission, pool, types, notify}.rs`.
- `acp/connection.rs` defines `AgentConnection` and `AgentRpc` traits.
  `LiveConnection` is the first `AgentConnection` impl;
  `ClientSideConnection` is the first `AgentRpc` impl. Worker handlers
  and session commands dispatch through the trait object (`Rc<dyn AgentRpc>`)
  so callers can drop `CONN_MAP` borrows before `.await`ing RPCs.

### Session ownership & terminals

- `initialize` advertises `meta.terminal_output = true`
  (`session/cold_start.rs:186-190`).
- Terminal stdout/stderr are drained in `spawn_local` background tasks
  (`client.rs:86-103`); `wait_for_terminal_exit` polls a
  `watch::Receiver` without holding the child lock.
- Terminal handles live on the owning `JockeyUiClient` as
  `Rc<RefCell<HashMap<terminal_id, TerminalHandle>>>`. `Drop for
  JockeyUiClient` reaps every remaining child, so reset / reconnect /
  idle reclaim cleans up terminals at the same moment as the connection.
- Terminal output keeps the tail, aligns drain cuts to the next newline,
  and derives `truncated` from `original_bytes_written`
  (`client.rs:63-103`).
- Every `acp::Client` handler on `JockeyUiClient` calls `validate_session`
  against the session id captured during cold-start
  (`client.rs:131-157`); mismatches return `InvalidParams`.

### State writeback and typed errors

- `LiveConnection` holds `mode_state: Rc<RefCell<SessionModeState>>` and
  `config_state: Rc<RefCell<Vec<SessionConfigOption>>>`. `JockeyUiClient`
  shares the same cells; `session_notification` writes back on
  `CurrentModeUpdate` and `ConfigOptionUpdate` without a worker
  round-trip. The worker's `SetMode` / `SetConfigOption` handlers apply
  optimistic updates and roll back on RPC failure.
- `AcpEvent::SessionError { code, message, retryable }` is emitted from
  every `handle_execute` error path before the final `result_tx`
  rejection. `session/execute.rs` captures the code during streaming
  and reuses it in `AcpPromptResult.error_code`. The string-reparsing
  `error_code_from_raw` helper has been deleted.
- `apply_cold_start_config` pre-validates saved `role_mode` against
  `mode_state.available_modes`, and emits `UNSUPPORTED_CONFIG` /
  typed-code session errors for skipped role config keys and failed
  RPC attempts, so the user sees why a saved value did not take effect.

### Cold-start dedup

- `thread_local! PENDING_COLD_STARTS: Shared<LocalBoxFuture<...>>` in
  `worker/pool.rs`. `ensure_connection` checks `CONN_MAP`, then
  `PENDING_COLD_STARTS`, and only spawns a new `cold_start` task if
  neither is present. Concurrent callers for the same `pool_key` share
  one agent subprocess and reuse the resulting `LiveConnection`.

### Frontend

- `AppSession` carries `terminals: Record<string, TerminalEntry>`,
  `pendingTerminalOutput: Record<string, string[]>`, and
  `lastError: SessionErrorInfo | null`.
- `acpEventBridge.ts` upserts `TerminalEntry` from
  `terminalMeta.terminalInfo`, buffers out-of-order `terminalOutput`
  chunks into `pendingTerminalOutput` until the matching info arrives,
  and sets `exitStatus` from `terminalExit`. It also handles the
  `sessionError` stream event.
- `ToolCallGroup` renders a monospace terminal view with a color-coded
  exit-code badge when a tool call's `terminalMeta.terminalInfo` is
  present; other tool calls keep the generic JSON view.
- `SessionErrorBanner` renders `AppSession.lastError` with recovery
  actions keyed off the typed `AcpErrorCode`: Reconnect for connection
  failures, Re-auth hint for `AUTH_REQUIRED`, Retry hint for
  `PROMPT_TIMEOUT`, and a universal Dismiss.
- Mode switcher in `SessionTabs` is a compact `<select>` instead of a
  row of toggle buttons.
- `acp_metrics_snapshot_cmd`, `acp_log_snapshot_cmd`, and
  `active_acp_connections_cmd` are registered Tauri commands exposing
  per-runtime latency, log tail, and live connection snapshots.

---

## Remaining work

### P1: Three-way mode sync (role default ↔ session override ↔ live agent)

Three "mode" surfaces exist today and can diverge:

- The top-right `<select>` in `SessionTabs` writes a per-session
  override via `set_acp_mode` (persisted in
  `app_session_roles.mode_override`). It calls `set_session_mode` RPC
  on the live connection and does optimistic-update-with-rollback.
- The Role editor in `ConfigDrawer` writes `roles.mode` (the global
  default) via `upsert_role`. It does **not** touch any live
  connection; changes only take effect on the next cold start of a
  session that has no `mode_override`.
- `apply_cold_start_config` resolves the effective mode as
  `mode_override ?? role.mode` when a connection is cold-started.

Divergence scenario: a user picks mode X in the dropdown for session A
(writes override=X), later edits the role default to Y, and opens a
new session B — A runs X, B runs Y, and editing the role again while
A is live has no visible effect on A.

#### Goal

When the user changes `Role.mode` in the editor, live sessions that
have no explicit `mode_override` should pick up the new default
without a restart.

#### Implementation tasks

- After `upsert_role` succeeds on a mode change, enumerate every live
  ACP connection for that role (via `active_acp_connections_cmd`
  internals or a new worker message).
- For each connection whose session has **no** `mode_override`
  recorded, send `set_session_mode(role.mode)` with the same
  optimistic-update-and-rollback discipline used by the dropdown.
- Update the in-memory `AppSession.currentMode` for those sessions so
  the dropdown reflects the new default immediately.
- Document the precedence rule in a short comment near
  `apply_cold_start_config` and the dropdown's `onChange`.

#### Acceptance criteria

- Editing a role's default mode updates every matching live session's
  dropdown and agent state within one round trip.
- Sessions that have an explicit override are unaffected.
- Forcing the `set_session_mode` RPC to fail rolls the live session
  back to its previous mode and surfaces a typed `SessionError`.

---

### P2: UI richness parity with Zed

Zed renders agent responses as a chat-first stream with specialized
widgets. Concretely, Jockey is missing: inline diffs with hunk
controls, an interactive PTY widget for agent terminals, collapsible
thinking blocks with a preview fade-out, clickable file paths that
hook into the existing preview tab system, and a "remember my choice"
affordance on permission prompts.

True CLI-native TUI interactivity (sending keystrokes back to the
agent-spawned process) is **out of scope**: ACP has no client → agent
terminal-stdin method. Keyboard input into the embedded terminal
would have to bypass ACP entirely, which defeats the "tool call"
model. The items below are ordered by value per unit of work.

#### P2-a: Clickable file paths → preview tab

Tool calls already carry `locations: [{ path, line? }]`. The
preview-tab infrastructure is already in `AppSession.previewTabs`.
Wiring is just a click handler that calls `setActivePreviewTabId` or
equivalent.

- Accept when: clicking a file path in a tool-call card opens (or
  focuses) a preview tab at that path, with `initialMode: "file"` or
  `"diff"` if the tool call is a write/edit.

#### P2-b: Collapsible thinking blocks

`AcpEvent::ThoughtDelta` already reaches the frontend via
`appendThought`. Render thought text in a dedicated collapsible
section per message with a preview fade-out, matching Zed's
`render_thinking_block` visual.

- Accept when: thoughts render in a distinct, collapsed-by-default
  section that can be expanded inline, with graceful behavior when
  thought text streams in after the main message already completed.

#### P2-c: Permission "remember my choice"

Backend `cached_approval` already exists. Extend the
`PermissionModal` to offer a "remember" checkbox that, when checked,
upgrades an AllowOnce response to AllowAlways so the cache picks it up
next time.

- Accept when: approving a tool call with "remember" ticked lets the
  agent perform the same kind of call again without a prompt until
  the session resets.

#### P2-d: Inline diff with hunk controls

When `ToolCall.content` carries a diff-like structure (patch text, or
a dedicated `ContentBlock::Diff` once the agents ship it), render via
a diff editor with `keep` / `reject` per hunk. On accept: noop (the
agent already wrote the file). On reject: open a follow-up prompt to
the agent asking it to revert the specific hunk.

- Accept when: a file-edit tool call displays hunks inline with
  controls, and rejecting a hunk produces a correct revert prompt.

#### P2-e: xterm.js terminal widget (read-only)

Replace the `<pre>` terminal view with an xterm.js instance so ANSI
colors, carriage returns, and the cursor position render correctly.
Still read-only — stdin cannot reach the agent's child process.

- Accept when: commands emitting color / progress characters render
  as they would in a real terminal; long output scrolls with the
  xterm viewport; memory stays bounded by the existing byte cap.

---

### P3: ACP session lifecycle commands

Add explicit `acp_list_sessions`, `acp_close_session`,
`acp_resume_session` Tauri commands, gated on the agent's
`session_capabilities.{list, close, load}` advertisement. Not needed
for any current user story; track as deferred until a concrete
scenario appears (session picker UI, per-session cleanup).

---

## Explicitly out of scope

- Ref-counted session store. Jockey's idle-reclaim
  (`worker/handlers.rs:74-99`) is the lifecycle driver; adding a
  ref_count layer for its own sake is premature.
- A separate `acp_thread` crate. Jockey's Tauri + Solid architecture
  consumes events directly from the worker; a second crate buys no
  test seam that the `AgentConnection` trait does not already
  provide.
- Changing the worker's current-thread + LocalSet model. It correctly
  handles `!Send` `Rc<ClientSideConnection>` without locks.
- Full TUI-style terminal interactivity (stdin injection into
  agent-spawned processes). Requires an ACP protocol extension.
- W3C trace-context propagation in session metadata — owned by
  `rfd_meta_propagation`.

---

## Cross-cutting validation

- Build: `cargo check --manifest-path src-tauri/Cargo.toml`,
  `cargo clippy --manifest-path src-tauri/Cargo.toml`,
  `pnpm exec tsc --noEmit`.
- Manual ACP (Codex / Gemini / Claude Code):
  - Normal prompt round-trip.
  - Long streaming command (`seq 1 100000 | pv -qL 2000`) — confirm
    `truncated=true` and the first surviving line is intact; confirm
    the terminal view shows a bounded monospace scroll.
  - Cancel during a running terminal command — confirm the child is
    reaped and a second `terminal_output` request returns
    `terminal not found`.
  - `reset_acp_session` immediately followed by `execute_runtime` —
    confirm no duplicate child pid via `active_acp_connections_cmd`.
  - Mode switch via the top-right dropdown — confirm optimistic UI
    update and rollback on a forced server error.
  - Config option switch — confirm `LiveConnection.config_state`
    updates without a restart.
  - Force-crash the agent process mid-prompt — confirm a typed
    `sessionError` appears, the banner renders with a Reconnect
    action, and the final rejection arrives after.
- Frontend stream continuity: open DevTools, run ten back-to-back
  prompts, confirm no `[acp/stream] seq gap` warnings.
- Concurrency stress: script ten `reset` + `execute` cycles;
  `child_pids()` must end empty.
