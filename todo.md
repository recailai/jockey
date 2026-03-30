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

### Theme & Window Chrome
- [ ] `titleBarStyle: "Overlay"` + `hiddenTitle: true` — 消除 title bar 与页面色差断层，融入页面背景
- [ ] 确定色彩方向：冷调蓝紫 (`#08080f` bg + `#7c6af5` accent) / 暖黑琥珀 / 玻璃态三选一
- [ ] 统一 CSS token：`--ui-accent` 引入彩色，替换当前纯白 accent
- [ ] 前端 `padding-top: env(titlebar-area-height, 28px)` 适配 Overlay 按钮区域
- [ ] Light mode 配套更新 accent 色（当前 light mode 无彩色 accent）
- [ ] 系统 dark/light 自动跟随 + 手动切换开关放入 Settings

### Terminal Visibility
- [ ] Expose `get_terminal_output(terminal_id)` and `kill_terminal_cmd(terminal_id)` as Tauri commands
- [ ] Frontend: detect `ToolCallContent::Terminal` (`type === "terminal"`) in ToolCallItem and render a live terminal panel
- [ ] Terminal panel: poll `get_terminal_output` on interval, show scrolling output, Stop button calls `kill_terminal_cmd`
- [ ] Note: ACP terminal = subprocess spawned by agent for bash commands (not a subagent); no stdin/human-input support in ACP spec
- [ ] Note: `kill_terminal` keeps TerminalId valid (agent can still read final output); `release_terminal` kills + invalidates TerminalId — Stop button should use kill, not release

### ACP Session Modes & Config Options (现状 + 缺口)

**Session Modes 现状：**
- ✅ 后端：`set_acp_mode` Tauri command 暴露，调用 `worker::SetMode` → `conn.set_session_mode()`
- ✅ 后端：`SessionUpdate::CurrentModeUpdate` → `AcpEvent::ModeUpdate { mode_id }` 已处理
- ✅ 后端：`SessionUpdate::AvailableCommandsUpdate` 已处理（注意：`AvailableModes` 是 cold_start 时从 `new_session` 响应里读取，不是 SessionUpdate）
- ✅ 前端：`SessionTabs` 渲染 available modes 按钮，点击调 `assistantApi.setMode()`
- ✅ 前端：`currentMode` badge 显示在消息头部
- ❌ 缺失：available modes 列表仅在 prewarm/cold_start 时获取一次，`session/update` 推送的 modes 变化未处理（`_ => return Ok(())` 丢弃）— spec 说 agent 可以在任何时候更新可用 modes
- ❌ 缺失：client 无法在 session 建立时通过 `newSession` 的 `initialMode` 字段指定初始 mode（目前是 cold_start 后再 `set_session_mode`，多一个 round-trip）

**Session Config Options 现状：**
- ✅ 后端：`set_acp_config_option` Tauri command 暴露
- ✅ 后端：`SessionUpdate::ConfigOptionUpdate` → `AcpEvent::ConfigUpdate { options }` 已处理
- ✅ 后端：`list_discovered_config_options_cmd` 暴露，prewarm 时从 `new_session` 响应里读取 config options
- ✅ 前端：`ConfigDrawer` 按 `category`（model/mode/other）分组显示 config options，支持 select 类型
- ⚠️ 部分：`ConfigOptionUpdate`（agent 主动推送更新）事件到达前端但未更新 ConfigDrawer 内的 select 状态（只更新了 session 的 configOptions 字段，UI 不响应）
- ❌ 缺失：agent 响应 `set_config_option` 时返回完整更新后的 config state（含 dependent changes），前端未读取响应并更新 UI

### ACP Content Types (部分未实现)
**后端 session_notification 现状：**
- `ContentBlock::Text` → ✅ 提取 `.text` 字段，发送 TextDelta
- `ContentBlock::ResourceLink` → ✅ 提取 `.uri` 作为文本显示（降级处理）
- `ContentBlock::Image` → ❌ `_ => return Ok(())` 静默丢弃，图片不显示
- `ContentBlock::Audio` → ❌ 同上，静默丢弃
- `ContentBlock::Resource` (EmbeddedResource) → ❌ 静默丢弃

**ToolCallContent 现状：**
- `ToolCallContent::Content(text/image/resource)` → ✅ 序列化为 JSON 存入 contentJson，文本可见
- `ToolCallContent::Terminal` → ❌ 序列化后前端无专门处理，terminal_id 不被识别（见 Terminal Visibility）
- `ToolCallContent::Diff` → ❌ 序列化后前端无专门处理，`{ type: "diff", path, oldText, newText }` 以原始 JSON 显示，无 diff 渲染

**Agent Plan 现状：**
- ✅ 后端：`SessionUpdate::Plan` → `AcpEvent::Plan { entries }` 已实现
- ✅ 前端：`currentPlan` 存入 session state，MessageWindow 渲染步骤列表（status 点 + content 文字）
- ⚠️ 缺失：`priority` 字段有类型定义但渲染时未使用；plan 不持久化到已完成消息里（只在 streaming 时显示）
- ❌ 缺失：plan 完成后不保存到 message segments，用户看不到历史 plan

**待做：**
- [ ] Image content：前端 ToolCallItem / session_notification 支持渲染 base64 图片（`data` + `mimeType`）
- [ ] Diff content：前端 ToolCallItem 识别 `type === "diff"`，渲染 unified diff 视图（path + old/new text）
- [ ] Plan 持久化：plan 完成后保存到 message segments，历史消息可回顾 plan
- [ ] Plan priority 显示：high/medium/low 用不同颜色或图标区分

### ACP Agent Session Management (未实现)
- [ ] `session/list` — list agent's known sessions (requires `sessionCapabilities.list`); useful for session picker on resume
- [ ] `session/load` — restore session by ID with full message history replay; prerequisite for session restore after restart
- [ ] `session/resume` — re-attach to session without replaying history (lighter than load); currently using `resume_session_id` field in `new_session` as workaround
- [ ] `session/close` — gracefully close session and free agent resources; currently gate-flagged as `unstable_session_close` in SDK v0.10.2
- [ ] `session/fork` — fork session for side tasks (summaries, PR descriptions) without polluting main history; gate-flagged as `unstable_session_fork`
- [ ] Note: `dump_session`/`destroy_session` do not exist in ACP spec — these names were mistaken; actual session lifecycle methods are load/resume/close/fork above

### Desktop Polish
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
