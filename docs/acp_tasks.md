# ACP Tasks

## Already landed

Verified complete against the current tree; kept here for traceability:

- Module split: `acp/{adapter, client, error, metrics, mod,
  runtime_state}.rs` + `session/{cold_start, execute, prewarm, mcp,
  session_cmds}.rs` + `worker/{mod, handlers, permission, pool, types,
  notify}.rs`.
- Terminal ACP capability alignment: `initialize` advertises
  `meta.terminal_output = true` (`session/cold_start.rs:186-190`);
  stdout/stderr drained in `spawn_local` background tasks
  (`client.rs:86-103`); `wait_for_terminal_exit` polls a
  `watch::Receiver` without holding the child lock
  (`client.rs:527-549`); `kill_terminal`, `release_terminal`, and
  `shutdown_terminals` reap child processes.
- Session request ownership validation: all nine `acp::Client` handlers
  in `JockeyUiClient` call `validate_session` against
  `expected_session_id` (`client.rs:131-157`); mismatches return
  `InvalidParams` with a structured log entry.
- Terminal metadata on tool calls: `AcpEvent::ToolCall` and
  `ToolCallUpdate` carry `terminal_meta` extracted from
  `meta.terminal_info / terminal_output / terminal_exit`
  (`client.rs:20-33, 316, 345`) and threaded through
  `AppToolCall.terminalMeta` in `acpEventBridge.ts:109, 163-174`.
- Metrics and live-connection introspection:
  `acp_metrics_snapshot_cmd`, `acp_log_snapshot_cmd`, and
  `active_acp_connections_cmd` are registered Tauri commands
  (`commands/runtime_cmd.rs:189-201`, `lib.rs:364-366`).
- Permission and cancel cleanup on reset/reconnect:
  `cancel_permissions_for` is invoked in both paths
  (`worker/handlers.rs:117, 139`).

---

## P0: Mode and config state writeback to `LiveConnection`

### Background

`LiveConnection.available_modes / current_mode` (`worker/pool.rs:31-33`)
are populated once during cold_start (`session/cold_start.rs:333-376`)
and never updated thereafter. `CurrentModeUpdate` and
`ConfigOptionUpdate` arrive via
`JockeyUiClient::session_notification` (`client.rs:354-363`) and are
forwarded to the frontend as `AcpEvent::ModeUpdate / ConfigUpdate`, but
the worker's authoritative state is not refreshed. `set_mode` and
`set_config_option` in `session/session_cmds.rs` send RPCs without
optimistic updates, so the UI briefly disagrees with the backend view
until the agent pushes an update back.

### Goal

Keep the worker's per-session mode and config state synchronized with
the agent, enable optimistic UI updates with rollback on RPC failure,
and avoid drift between backend snapshot queries and frontend state.

### Implementation tasks

- Add `mode_state: Rc<RefCell<acp::SessionModeState>>` and
  `config_state: Rc<RefCell<Vec<acp::SessionConfigOption>>>` fields to
  `LiveConnection`. Both live on the worker LocalSet; use `Rc<RefCell>`,
  not `Arc<Mutex>`.
- Pass clones of the same `Rc` cells into `JockeyUiClient` at
  construction so `session_notification` can write back without a
  round-trip through the worker channel.
- In `JockeyUiClient::session_notification`, after `validate_session`,
  mutate the appropriate cell on `CurrentModeUpdate` and
  `ConfigOptionUpdate` before forwarding the event to the delta channel.
- In the worker `SetMode` handler: snapshot the old mode id from
  `mode_state`, write the new id, send the RPC, roll back on error.
  Same pattern for `SetConfigOption` on the full
  `Vec<SessionConfigOption>`.
- Remove the now-redundant `available_modes` and `current_mode` fields
  from `LiveConnection`.

### Acceptance criteria

- After an agent-pushed `CurrentModeUpdate`,
  `active_acp_connections_cmd` reflects the new mode id without a
  restart.
- `set_mode` updates the cell before the RPC; a failed RPC rolls it
  back; a successful RPC leaves it as-is.
- `apply_cold_start_config` reads from the new cells rather than the
  removed `LiveConnection` fields.

---

## P0: Terminal output truncation on line boundaries

### Background

`append_terminal_output` (`client.rs:63-84`) keeps the most recent
bytes by draining the prefix when output exceeds `output_byte_limit`.
The drain cut is char-boundary-safe but not line-boundary-safe: the
first surviving line can begin mid-token. `TerminalState.truncated:
bool` is double-written with the state it represents.

### Goal

Preserve whole-line trailing output and derive the `truncated` flag
from a byte counter rather than storing it redundantly.

### Implementation tasks

- Replace `TerminalState.truncated: bool` with `original_bytes_written:
  u64` and compute `truncated = original_bytes_written > output.len()
  as u64` at response time in `terminal_output` (`client.rs:511-525`).
- In `append_terminal_output`, increment `original_bytes_written` by the
  incoming byte count. When over limit, advance the drain cut to the
  next `\n` after the char-boundary position so the first surviving
  byte begins a line.
- If the retained buffer contains no `\n`, fall back to the current
  char-boundary drain to still enforce the byte cap.

### Acceptance criteria

- A long command that exceeds `output_byte_limit` yields a snapshot
  whose first line is complete and whose `truncated` flag is `true`.
- A single huge no-newline line still respects the byte limit.
- Memory usage stays bounded by `output_byte_limit + one max line`.

---

## P0: Frontend terminal-backed tool call rendering

### Background

`AppToolCall.terminalMeta` carries `terminalInfo / terminalOutput /
terminalExit` verbatim through `acpEventBridge.ts:109, 163-174`. The UI
renders tool calls generically; terminal-backed calls appear as opaque
JSON rather than a streaming terminal view.

### Goal

Render tool calls whose `terminalMeta.terminalInfo` is present as a
monospace terminal view with accumulated output and an exit-status
badge.

### Implementation tasks

- In the session store, maintain a `Map<terminalId, TerminalEntry>`
  where `TerminalEntry` holds `{ label, cwd, output, exitStatus }`.
- In `applyAcpStreamEvent`:
  - On `toolCall` / `toolCallUpdate` with `terminalMeta.terminalInfo`,
    upsert the entry from the info block.
  - On `terminalMeta.terminalOutput` with a `data` chunk, append to
    `output`. If the entry does not yet exist, buffer the chunk in
    `pendingTerminalOutput[terminalId]` and flush when info arrives.
  - On `terminalMeta.terminalExit`, set `exitStatus`.
- Add a terminal-view branch to the tool-call renderer that activates
  when `toolCall.terminalMeta?.terminalInfo?.terminalId` is set; keep
  the JSON renderer as the fallback.

### Acceptance criteria

- An agent-initiated command like `git status` renders as a monospace
  stream with an exit-code badge, not a generic JSON tool call.
- Non-terminal tool calls render unchanged.
- Out-of-order `terminalOutput` chunks that arrive before
  `terminalInfo` are buffered and flushed when info arrives.

---

## P0: Terminal handle session cleanup

### Background

`TERMINAL_HANDLES` is a process-wide `DashMap<String, TerminalHandle>`
(`client.rs:46-49`) keyed only by terminal UUID.
`reset_worker_session` and `reconnect_worker_session` remove the
`LiveConnection` but leave any terminals created by the old agent
alive in this map until process shutdown. They are also the lookup
source for `terminal_output / wait_for_terminal_exit / kill /
release`, so a late request from a dead agent can still mutate the
global map.

### Goal

Terminals owned by a connection must be killed and their handles
dropped at the same moment as the connection.

### Implementation tasks

- Give each `JockeyUiClient` a private `terminals: Rc<RefCell<HashMap<
  String, TerminalHandle>>>` replacing the process-wide
  `TERMINAL_HANDLES` for that client.
- On `JockeyUiClient` drop (which happens when the
  `ClientSideConnection` drops, i.e. when `LiveConnection` is removed
  from `CONN_MAP`), iterate the map and call
  `terminate_terminal_process` on each handle.
- Keep a lightweight process-wide shutdown hook for the ACP shutdown
  path that walks all clients reachable via `CONN_MAP` before exit.

### Acceptance criteria

- Running a long terminal command, then `reset_acp_session`, kills the
  child process within a bounded window (no orphan pids visible via
  `pgrep`).
- A delayed `terminal_output` request for a dead session's terminal
  fails with `terminal not found`, not a stale snapshot.

---

## P1: `AgentConnection` trait extraction

### Background

`worker/handlers.rs` and `worker/mod.rs` dispatch via `WorkerMsg`
variants that all operate on a single concrete `LiveConnection`
(`worker/pool.rs:24-39`). There is no seam for alternate implementations
(tests, alternate transports, mocks). Zed's
`crates/acp_thread/src/connection.rs:47-188` uses an `AgentConnection`
trait for the same role.

### Goal

Introduce a trait-shaped agent connection so the worker loop depends on
a behavior contract rather than a concrete type, without changing the
current LocalSet + `Rc<ClientSideConnection>` execution model.

### Implementation tasks

- New file `src-tauri/src/acp/connection.rs` defining an object-safe
  trait `AgentConnection` with methods for the current live-connection
  operations: `execute_prompt`, `prewarm`, `cancel`, `reset`,
  `reconnect`, `set_mode`, `set_config_option`, plus accessors for
  `session_id`, `cwd`, `instance_id`, `health_rx`, and the new
  `mode_state` / `config_state` cells from the mode/config writeback
  task.
- Implement the trait on `LiveConnection` as the first implementation;
  no behavior changes.
- Change `worker/handlers.rs` and `session/session_cmds.rs` to hold
  `&dyn AgentConnection` (or `Rc<dyn AgentConnection>` where ownership
  must be shared) instead of `&LiveConnection`.
- Expose `AgentConnection` via `acp/mod.rs` so downstream modules and
  future tests can depend on the trait.

### Acceptance criteria

- `cargo check` and `cargo clippy` pass without functional change.
- The only call sites that still name `LiveConnection` directly are its
  construction in `cold_start.rs` and its insertion into `CONN_MAP`.
- A stub test implementation of `AgentConnection` can be compiled in a
  `#[cfg(test)]` module without pulling in the real agent subprocess.

---

## P1: Pending cold-start deduplication

### Background

`ensure_connection` (`worker/handlers.rs:170-215`) checks `CONN_MAP`
with an atomic borrow but then `.await`s `cold_start`. Two
`handle_execute` tasks spawned on the worker LocalSet for the same
`pool_key` can both observe an empty `CONN_MAP` and both proceed to
`cold_start`, spawning duplicate agent processes. `PROMPT_LOCKS` only
serializes the `conn.prompt(...)` phase and does not cover the
cold-start window.

### Goal

Ensure at most one cold_start is in flight per `pool_key`; concurrent
callers await and reuse the result.

### Implementation tasks

- Add `thread_local! static PENDING_COLD_STARTS: RefCell<HashMap<String,
  Shared<LocalBoxFuture<'static, Result<u64, String>>>>>` to
  `worker/pool.rs` (key: `pool_key`; inner `u64` is the `instance_id`
  of the eventually inserted connection).
- In `ensure_connection`:
  - If the pool key is in `CONN_MAP`, keep current behavior.
  - Else if in `PENDING_COLD_STARTS`, clone the shared future and await
    it. On `Ok`, re-fetch from `CONN_MAP` by `instance_id`.
  - Else insert a new shared task that runs `cold_start` and, on
    success, atomically inserts the new `LiveConnection` into
    `CONN_MAP` and removes itself from `PENDING_COLD_STARTS`. On
    failure, also remove itself before returning the error.
- No ref_count; idle reclaim (`worker/handlers.rs:74-99`) remains the
  lifecycle driver.

### Acceptance criteria

- Hammering `reset_acp_session` + `execute_runtime` in a tight loop
  never leaves extra child pids in `child_pids()`.
- Two concurrent `execute_runtime` calls for the same pool_key result
  in exactly one spawned agent process.

---

## P1: Typed session error events

### Background

`AcpLayerError` already carries a structured `AcpErrorCode` and
`retryable` flag (`error.rs:38-86`), but every error path serializes it
via `into_message()` to a plain string
(`worker/handlers.rs:307, 333, 553-609`,
`session/execute.rs:283`). The frontend re-parses the string via
`error_code_from_raw` (`session/execute.rs:382-411`) to recover the
code. There is no stream-level error event; the frontend only learns
about failure at the final `execute` promise rejection.

### Goal

Emit structured errors on the ACP event stream so the UI can render
distinct recovery actions (re-auth, retry, reconnect, view stderr).

### Implementation tasks

- Add `AcpEvent::SessionError { code: String, message: String,
  retryable: bool }` to `worker/types.rs`.
- In `handle_execute`, on every error branch
  (`worker/handlers.rs:553-609` and `Ok(Err(e))` in
  `session/execute.rs`), send an `AcpEvent::SessionError` through
  `delta_tx` before the `result_tx.send(Err(...))`.
- In `session/execute.rs`, forward `AcpEvent::SessionError` to
  `acp/stream` verbatim.
- In `src/lib/acpEventBridge.ts`, add a `sessionError` case that sets a
  session-level `lastError: { kind, message, retryable }`. UI
  components render action buttons based on `kind`:
  - `AUTH_REQUIRED`: "Re-authenticate"
  - `CONNECTION_FAILED` / `PROCESS_CRASHED`: "Reconnect" + "View stderr
    tail"
  - `PROMPT_TIMEOUT`: "Retry"
  - other codes: generic "Dismiss"
- Remove `error_code_from_raw` once the typed path is live.

### Acceptance criteria

- Forcing an auth failure shows a "Re-authenticate" action in the UI;
  the error code in DevTools is `AUTH_REQUIRED`, not a reparsed string
  match.
- Killing the agent process mid-prompt emits a `sessionError` event
  before the final execute rejection; the UI doesn't wait for the
  rejection to show the error banner.

---

## P2: Visible cold-start config application warnings

### Background

`apply_cold_start_config` (`worker/handlers.rs:217-286`) reads a role's
configured default mode and config values and applies them. When a
saved value isn't supported by the current runtime
(`handlers.rs:251-255`) it is silently skipped; the user gets no signal
that their configuration was ignored.

### Goal

Surface config application failures so the user knows why their saved
mode or config option did not take effect.

### Implementation tasks

- Extend the typed error event from P1 with a warning-severity variant
  or emit `AcpEvent::SessionError { code: "UNSUPPORTED_CONFIG",
  retryable: false, .. }` with a descriptive message for each skipped
  value.
- Log the skipped value through `acp_log("config.unsupported", ..)`.

### Acceptance criteria

- Switching a role to a runtime that lacks the saved `role_mode`
  produces a visible warning in the UI.
- `acp_log_snapshot_cmd` shows a `config.unsupported` entry for the
  skipped value.

---

## Cross-cutting validation

- Build: `cargo check --manifest-path src-tauri/Cargo.toml` and
  `cargo clippy --manifest-path src-tauri/Cargo.toml`.
- Manual ACP against Codex, Gemini, and Claude Code:
  - Normal prompt round-trip.
  - Long streaming command (e.g. `seq 1 100000 | pv -qL 2000`) —
    confirm `truncated=true` and the first visible line is intact.
  - Cancel during a running terminal command — confirm the child is
    reaped and `terminal_output` returns `terminal not found`.
  - `reset_acp_session` then immediate `execute_runtime` — confirm no
    duplicate child pid via `active_acp_connections_cmd`.
  - Mode switch via `set_acp_mode` — confirm optimistic UI update and
    rollback on a forced server error.
  - Config option switch — confirm `LiveConnection.config_state` is
    updated without a restart.
  - Force-crash the agent process mid-prompt — confirm a typed
    `sessionError` appears before the final rejection.
- Frontend stream continuity: open DevTools, run ten back-to-back
  prompts, confirm no `[acp/stream] seq gap` warnings.
- Concurrency stress: script ten `reset` + `execute` cycles;
  `child_pids()` must end empty.

## Out of scope for this pass

- No explicit `session/close`, `session/list`, or `session/resume`
  Tauri commands — the existing saved-session-id auto-resume path
  stays.
- No ref-counted session store; idle reclaim remains the lifecycle
  driver.
- No separate `acp_thread` crate — Jockey consumes events directly
  from the worker.
- No changes to the worker's current_thread + LocalSet model — it
  correctly handles `!Send` `Rc<ClientSideConnection>` without locks.
- No W3C trace-context propagation (owned by `rfd_meta_propagation`).
