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

### P1.6: Zed-style "+" menu + settings surface redesign

#### Background

Zed's agent panel exposes an "+" dropdown (MCP Servers / Rules /
Profiles / Settings / Reauthenticate / Toggle Threads Sidebar) and a
Settings page organized as External Agents / MCP Servers / LLM
Providers. Jockey's equivalent is the gear icon in `SessionTabs`
opening `ConfigDrawer`, which conflates theme, assistant runtime
selection, and role list.

Key differences from Zed's model, per the design decisions captured
for this task:

- **Role is top-level and global** (cross-project). Do not split into
  separate `ExternalAgent` + `Profile` tables; the existing `roles`
  table already captures the full "profile" bundle. Skip Zed's
  Profiles menu item entirely.
- **MCP servers become a global pool** with per-role enable/disable.
  Today MCP is embedded in `Role.mcpServersJson`; the refactor
  introduces a separate `mcp_servers` table and a join table
  `role_mcp_servers (role_id, mcp_server_id, enabled)`, and
  eventually supports **dynamic enable/disable on a live session**
  without cold restart.
- **Four independent concepts all coexist**: `systemPrompt`,
  `rules`, `skills`, `mcpServers`. They are not aliases or
  rebrandings of each other. Verified via
  `src-tauri/src/chat/prompt_builder.rs:24-30`: the existing
  builder only knows about `systemPrompt`. Rules are a new
  concept layered on top.
  - **`systemPrompt`** (existing, per-role): the always-injected
    "System:\n…" block at the start of every prompt. Unchanged.
  - **`rules`** (new, global with per-role selection): a library
    of named, reusable rule entries (each is name + content
    markdown). A role can enable any subset of rules; enabled
    rules are concatenated into the prompt after
    `systemPrompt`. Independent of `Role.systemPrompt` so the
    same rules can be shared across roles.
  - **`skills`** (existing `AppSkill`): user-triggered via
    `@mention` in chat, inserted into the message text at send
    time. Not auto-injected.
  - **`mcpServers`** (existing, migrated to global pool): tool
    providers attached to a session at cold-start.
  All four are surfaced in the "+" menu and in Settings as
  separate sections.
- **Drop Zed's Reauthenticate menu item.** The
  `SessionErrorBanner` `Reconnect` action already covers the same
  ground.

#### Goal

Replace the gear icon with a richer "+" menu that consolidates
session-scoped actions (MCP, Rules, Skills, Settings) and redesign
`ConfigDrawer` (or replace it with a new Settings page) so the
sections match Zed's External Agents / MCP Servers / LLM Providers /
Rules / Skills layout, without changing Jockey's semantics (role is
top-level, MCP is a global pool).

#### Implementation tasks

##### P1.6-a: "+" menu component

- New component `src/components/AgentMenu.tsx`. Dropdown anchored to
  a button placed next to (or replacing) the gear in `SessionTabs`.
- Menu items — designed to be extension-friendly; each item is an
  entry in a typed `AgentMenuItem[]` array so future items are a
  one-line addition:
  ```ts
  type AgentMenuItem = {
    id: string;
    label: string;
    section?: string;
    shortcut?: string;  // "⌥⌘M" etc.
    icon?: JSX.Element;
    onSelect: () => void;
    when?: () => boolean;  // conditional visibility
  };
  ```
- Initial items:
  - `MCP Servers → View all`            (⌥⌘M) opens MCP page
  - `MCP Servers → Add custom server…`  opens quick-add modal
  - `Rules → Manage`                    (⌥⌘L) opens rules library
  - `Rules → Add rule…`                 opens quick-add rule modal
  - `Skills`                            (⌥⌘K) opens skills manager
  - `System Prompt`                     (⌥⌘P) jumps into the active
                                        role's `systemPrompt` editor
  - `Roles`                             opens roles manager
  - `Settings`                          (⌥⌘,) opens settings page

##### P1.6-b: Quick-add MCP server modal

- New component `src/components/QuickAddMcpServerModal.tsx`.
- Transport switch: stdio / http / sse. Form fields from
  `src/components/management/primitives.tsx` (`McpServerStdio`,
  `McpServerHttp`, `McpServerSse`).
- Submit writes to the **global `mcp_servers` table** (see data
  model below) and, optionally via a "Enable for this role"
  checkbox (default on), adds a row to `role_mcp_servers`.
- Shared form extracted into
  `src/components/mcp/McpServerForm.tsx` so the modal, the settings
  page, and the roles page all reuse it.

##### P1.6-c: Data model — extract global MCP pool

- New SQLite tables (in `src-tauri/src/db/mod.rs`):
  ```
  mcp_servers(
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE,
    transport TEXT CHECK IN ('stdio','http','sse'),
    config_json TEXT NOT NULL,   -- transport-specific payload
    created_at INTEGER, updated_at INTEGER
  )
  role_mcp_servers(
    role_id TEXT NOT NULL REFERENCES roles(id),
    mcp_server_id TEXT NOT NULL REFERENCES mcp_servers(id),
    enabled INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(role_id, mcp_server_id)
  )
  ```
- Migration: on schema version bump, read every
  `roles.mcp_servers_json`, upsert each entry into `mcp_servers`
  (dedup by name), populate `role_mcp_servers`. Leave
  `roles.mcp_servers_json` in place for one release cycle as a
  rollback safety net; stop reading it once the migration runs.
- Tauri commands: `list_mcp_servers`, `upsert_mcp_server`,
  `delete_mcp_server`, `set_role_mcp_enabled(role_id,
  mcp_server_id, enabled)`.

##### P1.6-c.2: Rules library data model + prompt builder change

- New SQLite tables:
  ```
  rules(
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    content TEXT NOT NULL,        -- markdown
    description TEXT,
    created_at INTEGER, updated_at INTEGER
  )
  role_rules(
    role_id TEXT NOT NULL REFERENCES roles(id),
    rule_id TEXT NOT NULL REFERENCES rules(id),
    enabled INTEGER NOT NULL DEFAULT 1,
    ord INTEGER NOT NULL DEFAULT 0,  -- ordering within the role
    PRIMARY KEY(role_id, rule_id)
  )
  ```
- Tauri commands: `list_rules`, `upsert_rule`, `delete_rule`,
  `set_role_rules(role_id, Vec<(rule_id, enabled, ord)>)`.
- Prompt builder (`src-tauri/src/chat/prompt_builder.rs`): extend
  signature to take `enabled_rules: &[(String /*name*/, String
  /*content*/)]`. After the existing `System:\n{systemPrompt}`
  block, append each rule as:
  ```
  Rule: {name}
  {content}
  ```
  with a blank line between rules, then the existing `Context:` /
  `User:` blocks. Order respects `role_rules.ord`.
- `session_runtime.rs`: load enabled rules for the role alongside
  the existing `role_system_prompt` lookup and thread them through
  to `build_prepared_prompt`.
- A rule's content is **not** automatically reloaded on an
  existing session — rules apply on cold-start only (no ACP hot
  update for system-level prompt). Changing the enabled rule set
  for an active role triggers the same reset-on-next-prompt
  behavior as MCP changes (P1.6-d).

##### P1.6-d: Dynamic MCP enable/disable on live session

- Today MCP servers are only applied at cold-start (`new_session` /
  `load_session` carries `mcp_servers: Vec<McpServer>`). ACP does
  not have a "hot-swap MCP" method on an open session. Deal with
  this as two phases:
  - Phase 1 (easy): toggling enabled-state for a role in the UI
    triggers a `reset_acp_session` on any live session bound to
    that role so the next prompt cold-starts with the new set.
    Surface a one-line banner "MCP servers changed — reconnecting"
    so the user knows why the agent restarted.
  - Phase 2 (protocol): watch the ACP spec for a `session/mcp_update`
    capability; gate phase 2 behind capability advertisement and
    fall back to phase 1 when absent.

##### P1.6-e: Settings page redesign

- Replace the current `ConfigDrawer` with a structured Settings
  page laid out by section (matches Zed's screenshot). Sections,
  in order:
  - **External Agents** — read-only health list of detected
    runtimes (Claude Code / Codex / Gemini CLI), from
    `detect_assistants`. No add/remove UI at this level (runtimes
    are tied to installed binaries).
  - **MCP Servers** — global pool management. List, add, edit,
    delete. Each row shows how many roles reference it.
  - **Rules** — global rules library. List, add, edit, delete
    rule entries. Each row shows how many roles reference it.
  - **Skills** — reuse existing `SkillsTab` content.
  - **Roles** — existing `RolesTab` content, plus per-role:
    - `systemPrompt` editor (textarea, unchanged)
    - MCP multi-select (from global pool, each with enable toggle
      and optional order)
    - Rules multi-select (from global library, each with enable
      toggle and order)
    Inline add forms for MCP / Rules from this page use the same
    quick-add modals as the "+" menu so behavior is consistent.
  - **Interface** — theme (unchanged).
- Keep the drawer layout if the current right-slide panel has
  muscle memory value; otherwise promote to a full-page route.

##### P1.6-f: Extensibility abstractions

- `AgentMenuItem[]` registry described in P1.6-a makes menu items
  trivially extensible.
- Add a `McpTransportKind = 'stdio' | 'http' | 'sse'` string-union
  central to `primitives.tsx`, so adding a new transport (e.g.
  future `websocket`) is one extension point not a scattered switch.
- The Settings page uses a `SettingsSection[]` array driven
  rendering so new sections (future: LLM Providers, Telemetry, …)
  are a push, not a rewrite.

#### Acceptance criteria

- Clicking "+" opens the dropdown; all listed items navigate or
  open modals correctly.
- Adding an MCP server via the quick-add modal persists to the
  global `mcp_servers` pool and (when the checkbox is on) enables
  it for the active session's role via `role_mcp_servers`.
- Adding a rule via the quick-add modal persists to the global
  `rules` library and (when the checkbox is on) enables it for
  the active session's role via `role_rules`.
- Toggling a rule's enabled state for a role changes the content
  of the next cold-started prompt (verifiable by inspecting
  `acp_log_snapshot_cmd` after a prompt) without the other
  existing rules' order being disturbed.
- Changing either the enabled MCP set **or** the enabled rules set
  on a role triggers a `reset_acp_session` for any live session
  bound to that role, with a one-line banner explaining the
  reconnect.
- Renaming a global MCP server or rule propagates to every role
  that references it (name is not duplicated per role).
- Deleting a role does not delete any global MCP server or rule
  (only deletes join-table rows).
- A new menu item can be added by appending one entry to
  `AgentMenuItem[]` and not modifying the menu component.
- A new Settings section can be added by appending one entry to
  `SettingsSection[]` and not modifying the Settings page shell.
- `cargo check`, `cargo clippy`, `pnpm exec tsc --noEmit` all clean.

---

### P1.5: Turn-control UX (queue, interrupt-and-send, approval flow)

Zed delivers "feels like a TUI" interactivity with just three ACP
primitives: `prompt`, `cancel`, and `request_permission`. A
user-visible investigation of Zed confirmed there is no stdin /
keystroke channel back to the agent; the sense of interactivity comes
entirely from frontend turn-taking state. Jockey already has the
permission path; it is missing the queue and the interrupt-and-send
paths.

#### P1.5-a: Auto-send queued messages

`AppSession.queuedMessages` is populated when the user hits send
while the agent is running, but nothing drains it today. Zed's model
(`crates/agent_ui/src/conversation_view.rs:1530-1554`): after
`prompt()` resolves the UI checks the queue and auto-sends the next
entry.

- Implementation: watch the session's running-state transition to
  idle in `useStreamEngine` / `streamSession.ts`; if
  `queuedMessages` is non-empty, shift the head and feed it to the
  regular submit path.
- Surface a queue badge and per-entry remove / edit-before-send
  affordances on the message row above the input box. The "Queued"
  panel that already exists in `MessageWindow.tsx:369` is the
  anchor point.

Accept when: typing three messages rapid-fire while the agent runs
results in all three being processed in order, with a visible queue
count and the ability to cancel any still-queued entry.

#### P1.5-b: Interrupt-and-send ("jump the queue")

Zed maps this to `cancel` + fresh `prompt` on the next turn. Jockey
has `assistantApi.cancelSession` but no combined path that also
carries the new prompt as the next turn.

- Implementation: add a "send now" variant on the chat input that,
  when the session is running, awaits `cancelSession` (already
  bounded 5s in the worker), drains any partial stream state, and
  then submits the message as a normal prompt. On the backend this
  is just `WorkerMsg::Cancel { result_tx }` then the usual
  `WorkerMsg::Execute`.
- UX: show a chip next to the send button during a running turn that
  toggles between "Queue" (default) and "Send now" (interrupts). A
  keyboard shortcut (Cmd+Shift+Enter) is the natural fit.

Accept when: hitting "Send now" while the agent is mid-turn cancels
the current turn within a bounded window and the new message is
processed as the next turn; queued messages remain queued.

#### P1.5-c: In-stream permission status

Today the `PermissionModal` is a modal overlay. Zed keeps the
permission UI inline with the tool call card so the rest of the
stream stays visible. Jockey's `AcpEvent::PermissionRequest` already
flows through, and every tool call that needs approval already has a
matching `ToolCall` stream event.

- Implementation: correlate the permission request with the most
  recent in-flight `ToolCall` (same `appSessionId`, tool status is
  `pending`) and render the approval buttons inside the tool call
  card in `ToolCallGroup.tsx`. Keep the modal as a fallback when no
  correlation exists.

Accept when: a permission prompt for a tool call shows the approve /
deny buttons directly under the tool call card, with no modal, and
resolves the same oneshot on the backend.

---

### P2: UI richness parity with Zed

Zed renders agent responses as a chat-first stream with specialized
widgets. Concretely, Jockey is missing: inline diffs with hunk
controls, an ANSI-capable terminal widget for agent terminals,
collapsible thinking blocks with a preview fade-out, clickable file
paths that hook into the existing preview tab system, and a
"remember my choice" affordance on permission prompts.

True CLI-native TUI interactivity — sending raw keystrokes back into
the agent-spawned child process's stdin — remains out of scope: ACP
has no client → agent terminal-stdin method. (See P1.5 for the
items that give the *perception* of TUI-level control without needing
a new protocol channel.) The items below are ordered by value per
unit of work.

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
- Raw stdin injection into agent-spawned terminal child processes.
  No ACP method exists for it; Zed does not implement it either. The
  user-facing "feels like a TUI" experience (queue, interrupt, inline
  approvals) is covered by P1.5 and does not need this channel.
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
