# Jockey Pending Tasks

This document consolidates all outstanding tasks, roadmaps, and pending features across the Jockey multi-agent orchestration platform.

## High-Priority Integration Tasks

### P1: Three-Way Mode Sync (Role Default ↔ Session Override ↔ Live Agent)
Three mode surfaces exist today (SessionTabs dropdown, Role Editor in ConfigDrawer, and `apply_cold_start_config`). They can diverge when role configurations are updated mid-session.
- [ ] Enumerate every live ACP connection for that role after `upsert_role` succeeds on a mode change.
- [ ] Send `set_session_mode(role.mode)` to any connection without an explicit session-level `mode_override`.
- [ ] Update in-memory `AppSession.currentMode` for affected sessions.
- [ ] Document precedence rules near `apply_cold_start_config` and dropdown's `onChange`.

### P1.5: Turn-Control UX (Queue, Interrupt, Inline Approvals)
- [ ] **P1.5-a: Auto-send queued messages**: Watch running-state transitions to idle in `useStreamEngine`/`streamSession.ts` and auto-submit the next queued message. Add remove/edit controls to the queue row.
- [ ] **P1.5-b: Interrupt-and-send**: Add a "Send now" variant that triggers `cancelSession` before executing the new prompt (bound to Cmd+Shift+Enter).
- [ ] **P1.5-c: Inline permission status**: Refine the correlation of permission requests with the active pending `ToolCall` to render approval buttons directly in the tool call card instead of a modal dialog.

### P1.6: Zed-Style "+" Menu & Settings Redesign
- [ ] **P1.6-a: "+" menu component**: Create `src/components/AgentMenu.tsx` dropdown containing shortcuts to MCP, Rules, Skills, System Prompt, and Settings.
- [ ] **P1.6-b: Quick-add MCP server modal**: Add quick-add modal form reusing stdio/http/sse transport primitives.
- [ ] **P1.6-c: Data model — extract global MCP pool**: Implement global `mcp_servers` and `role_mcp_servers` join tables in SQLite. Implement Tauri commands to CRUD global servers.
- [ ] **P1.6-c.2: Rules library data model + prompt builder integration**: Create `rules` and `role_rules` tables. Modify prompt builder to inject enabled rules after `systemPrompt`.
- [ ] **P1.6-d: Dynamic MCP enable/disable**: Dynamically swap/reconnect ACP sessions when role bindings change.
- [ ] **P1.6-e: Settings page redesign**: Replace `ConfigDrawer` with a multi-section settings page containing: External Agents, MCP Servers, Rules, Skills, Roles, and Interface.
- [ ] **P1.6-f: Extensibility abstractions**: Support generic array registries (`AgentMenuItem[]`, `SettingsSection[]`) to simplify adding future options.

---

## UI Shell Migration & Visual Polish

### Remaining UI Design System Migration (from ui_design_system_migration.md)
- [ ] **Reduce legacy `theme-*` usage**: Clean up old styling classes in management subpages.
- [ ] **Searchable select primitive**: Replace custom branch pickers with full dropdown/combobox primitives.
- [ ] **Token cleanup**: Collapse duplicate CSS token blocks into a single ordered section in `index.css`.

### P2: UI Richness Parity
- [ ] **P2-a: Clickable file paths**: Open/focus matching preview tabs from location links in tool-call cards.
- [ ] **P2-b: Collapsible thinking blocks**: Render thought stream in collapsed-by-default expandable panels with fade-out.
- [ ] **P2-c: Permission "remember choice"**: Add checkbox to upgrade AllowOnce to AllowAlways in database cache.
- [ ] **P2-d: Inline diff with hunk controls**: Render diffs inline with keep/reject hunk buttons. Hunk rejection triggers revert prompt.
- [ ] **P2-e: xterm.js terminal widget**: Replace `<pre>` terminal view with an active xterm.js instance for ANSI escape color support.

---

## Core Product Roadmap

### Assistant & Role Core
- [ ] **Assistant abstraction layer**: Persistent configurable assistant entities.
- [ ] **Role improvements**: Binding conflict warnings, inline editing, assistant binding.
- [ ] **Active role context tracking**: Shell-prompt style input and automatic message routing.
- [ ] **Separate commands**: Split app commands vs agent commands (plan/act/auto/cancel) dispatched via ACP.
- [ ] **Cross-role context sharing**: Message channels and automatic summary injection.
- [ ] **Error state handling**: Implement retry UI and graceful errors instead of unconditional "done" state.
- [ ] **Session-scoped input**: Isolate queued inputs and history per session.

### Workflow Engine (Automations)
- [ ] **Workflow progress UI**: Step cards and status indicators.
- [ ] **Workflow cancellation**: Terminate entire workflow runs gracefully.
- [ ] **Structured step handoff**: Support typed payload handoffs instead of plain text concatenation.
- [ ] **Conditional branching**: Conditional steps and parallel execution tracks.
- [ ] **Human-in-the-loop**: Add approval steps in workflows.
- [ ] **Per-step configuration**: Timeout, retries, and custom templates.

### Session & Context Management
- [ ] **Persistent chat sessions**: Session restore after app restart and message history indexing.
- [ ] **Session profiles**: Presets for Cwd, runtime, modes, and MCP server configuration.
- [ ] **Lazy-load sessions**: Load metadata first, then stream details on demand.
- [ ] **Layered context scopes**: Global, assistant, role, session, and workflow scopes.
- [ ] **Auto-cleanup**: Clear context entries on session end.
- [ ] **Context panel**: Group context entries by scope in the sidebar.

### Capability Registry
- [ ] **Registry tables**: `mcp_registries` + `skill_registries` with endpoint/auth/priority.
- [ ] **Cache tables**: `remote_mcp_servers` + `remote_skills` local caches.
- [ ] **Sync logic**: Pull and upsert caches from registries (`capability_sync.rs`).
- [ ] **Structured bindings**: Map `role_capability_bindings` to replace raw JSON structures.
- [ ] **Catalog picker**: Merged registry catalogs with conflict indicators in the role editor.
- [ ] **Session overrides**: Session-level capability add/disable delta tables.

### ACP Capabilities & Protocol Alignment
- [ ] **Rich content blocks**: Support rendering base64 images, audio, and embedded resources.
- [ ] **ToolCall terminal/diff content**: Identify `type === "terminal"` and `type === "diff"` for dedicated rendering.
- [ ] **Plan improvements**: Plan priority badges, plan persistence, and historical segments lookup.
- [ ] **Session lifecycle protocols**: Support `session/list`, `session/load`, `session/resume`, `session/close`, and `session/fork` protocols in backend/frontend.
- [ ] **Client registry PR**: Submit PR to add JockeyUI to ACP client list.

### Desktop & Polish
- [ ] **Performance optimizations**: Batch Tauri events, permission cache TTL, replace subqueries with JOINs, virtual scrolling, skeleton loaders.
- [ ] **System integration**: Background throttling config, high-risk command confirms, fix PR browser launch.
- [ ] **Aesthetics**: Frosted glass, glow effects, floating command bars, smooth entry animations.
