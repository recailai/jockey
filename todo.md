# Roadmap

## In Progress

- [ ] Assistant abstraction layer (persistent assistant entities with configurable model, mode, and MCP servers)
- [ ] Role improvements (bind conflict warnings, inline editing, assistant binding)
- [ ] Active role context tracking (shell-prompt style input, automatic message routing)
- [ ] Separate app commands vs agent commands (plan/act/auto/cancel dispatched via ACP)
- [ ] Cross-role context sharing (per-role message channels, automatic summary injection)
- [ ] Error state handling (proper error status instead of unconditional "done", retry UI)
- [ ] Session-scoped input state (isolate queued inputs and history per session)

## Planned

### Workflow Engine
- [ ] Workflow progress UI (step cards, progress indicators)
- [ ] Workflow cancellation (cancel entire workflow, not just individual steps)
- [ ] Structured step-to-step handoff (replace text concatenation with typed payloads)
- [ ] Conditional branching and parallel step execution
- [ ] Human-in-the-loop approval steps
- [ ] Per-step configuration (prompt templates, timeouts, retries)

### Session Management
- [ ] Persistent chat sessions with message history
- [ ] Session restore after app restart
- [ ] Session configuration profiles (reusable presets for cwd, runtime, mode, MCP servers)
- [ ] Lazy-load session messages on switch (load metadata first, messages on demand)

### Context System
- [ ] Layered context scopes (global, assistant, role, session, workflow)
- [ ] Automatic session-context cleanup on session end
- [ ] JSON value support for context entries
- [ ] Context panel grouped by scope in the UI

### MCP Integration
- [ ] Parse MCP servers config and pass to runtime execution
- [ ] MCP servers editor in role configuration form
- [ ] MCP-over-ACP tool injection for cross-role queries (long-term)

### Capability Registry
- [ ] `mcp_registries` + `skill_registries` tables — multi-source subscription model, each with endpoint/kind/auth/sync_mode/priority
- [ ] `remote_mcp_servers` + `remote_skills` tables — local cache keyed by registry_id
- [ ] `capability_sync.rs` — HTTP/file pull for registry endpoints, upsert cache on sync
- [ ] `role_capability_bindings` table — replace `roles.mcp_servers_json` raw JSON with structured bindings (cap_kind, cap_id, override_json)
- [ ] `session_runtime.rs` — assemble `mcp_servers` + skill context_pairs from bindings instead of inline JSON
- [ ] Startup auto-sync for all enabled registries
- [ ] Settings UI: Registry list (MCP + Skill), add/remove/refresh per registry
- [ ] Role editor: replace MCP JSON textarea with catalog picker (shows merged view from all registries, annotated by source)
- [ ] Conflict display: same-name entries from different registries shown separately, user picks which binding to use
- [ ] `role_capability_bindings.enabled` field — bind without activating (default on/off per role)
- [ ] `app_session_role_cap_overrides` table — session-level add/disable delta over role defaults; final inject set = role bindings + session.added - session.disabled

### Performance

#### Backend (Rust)
- [ ] P2: Batch Tauri event emissions — collect events per frame/interval instead of per-delta `app.emit()` (session.rs:449-497)
- [ ] P2: Add TTL / cleanup for `PERMISSION_REQUESTS` DashMap — abandoned entries leak (worker.rs:83-95)
- [ ] P2: Replace app_session loading subquery with JOIN (app_session.rs:120)
- [ ] P3: Cache `pool_key()` / `commands_key()` to avoid repeated `format!()` allocation (worker.rs:269-275)
- [x] P0: Hoist `load_recent_role_chats` out of role loop — N+1 query (chat.rs:289-348)
- [x] P1: Eliminate double serialization — use `#[derive(Serialize)]` payload structs for `app.emit()` (session.rs)
- [x] P1: Bound delta channel — `mpsc::channel(512)` replaces `unbounded_channel` (worker.rs, client.rs, session.rs)

#### Frontend (SolidJS)
- [ ] P2: Index `toolCallId → segmentIndex` for O(1) `toolCallUpdate` lookup (App.tsx:1183)
- [ ] Virtual scrolling for long message lists
- [ ] Skeleton loading screen on startup
- [ ] Lazy-load and code-split configuration drawer
- [ ] Batch initial Tauri invoke calls on mount
- [ ] Move synchronous directory reads to background threads
- [ ] Preload fonts and CSS to reduce flash of unstyled content

### Desktop Polish
- [ ] Native macOS window chrome (traffic light positioning, hidden title bar)
- [ ] Background throttling configuration
- [ ] Confirmation prompts for high-risk commands

### UI/UX Refinements
- [ ] Dark-mode visual refresh with depth and elevation
- [ ] Frosted-glass panel styling
- [ ] Glow effects on interactive elements
- [ ] Floating command bar input
- [ ] Smooth message entrance animations

## Completed

- [x] Role model redesign (model, mode, MCP servers, auto-approve fields)
- [x] Database migration for extended role schema
- [x] Role CRUD with full field support
- [x] Role copy and edit commands
- [x] Frontend role creation form with all fields
- [x] Remove hardcoded adapter versions and defaults
- [x] Platform-aware PATH construction
- [x] Complete ACP protocol implementation (all 11 event variants)
- [x] Permission request handling (auto-approve and one-shot)
- [x] Full ACP client trait implementation (filesystem and terminal)
- [x] MCP servers passthrough and streaming events
- [x] Session lifecycle commands (cancel, set mode, set config)
- [x] Frontend mode switching UI
- [x] Codebase modularization (split monolithic files into focused modules)
- [x] ACP stream protocol alignment (proper event unwrapping in frontend)
- [x] Session isolation fix (per-app-session agent connections, no cross-talk)
- [x] Connection pool keyed by app session ID
- [x] Streaming buffer optimization (direct emit, RAF batching in frontend)
- [x] Tool call persistence in messages
- [x] Two-phase prewarm strategy (recent session first, then remaining roles)
- [x] Thought deltas routed to thinking indicator instead of message body
