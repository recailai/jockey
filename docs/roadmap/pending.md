# Jockey Pending Tasks

Open items only. Fixed items live in [../reviews/](../reviews/review-2026-10.md); shipped features are listed in [../archive/completed-tasks.md](../archive/completed-tasks.md). Items marked *(unverified)* were not traced in code.

## Turn control and composer
- [ ] **Cmd+Shift+Enter "send now"**: no shortcut yet. The backend `send_queued_now` exists (`src-tauri/src/engine/service.rs`, used by `web_gateway/api.rs`).
- [ ] **"+" agent menu**: `AgentMenu.tsx` does not exist; only `src/components/composer/AgentPicker.tsx`. Decide whether a Zed-style menu (MCP, Rules, Skills, Settings shortcuts) is still wanted.
- [ ] **Clickable file paths**: open/focus the matching preview tab from tool-call locations (`ToolCallGroup.tsx`).
- [ ] **Rich content blocks**: only images are rendered; audio and embedded resources are not.

## Sessions, roles and context
- [ ] **Dynamic MCP swap**: changing a role's MCP bindings does not reconnect a live session (`acp/session/mcp.rs`).
- [ ] **Mode-sync precedence docs**: write down role default vs session override (`mode_override`) vs live agent precedence near `apply_cold_start_config` and the mode dropdown. Behaviour itself is covered by review item D6.
- [ ] **Cross-role context sharing**: only `RECENT_ROLE_CHATS_KEY` (`src-tauri/src/chat.rs`) exists; no channels or summary injection.
- [ ] **Error/retry UI** (partial): `SessionErrorBanner.tsx` and recorded failed turns exist; there is no retry action.
- [ ] **Session profiles, layered context scopes, auto-cleanup, context panel grouped by scope** *(unverified)*.

## Workflows ("Automations")
- [ ] **Decide the feature's fate**: workflows can be created (`WorkflowsTab.tsx`) but `start_workflow` (`db/session.rs`, registered in `lib.rs`) has no frontend caller. Wire it up or remove the feature.
- [ ] If kept: progress UI, cancellation, typed step handoff, branching, human-in-the-loop steps, per-step timeout/retry *(unverified)*.

## Capability registries
- [ ] Re-scope against the existing `management/McpRegistryTab.tsx` and `SkillRegistryTab.tsx`: registry/cache tables, sync, structured role bindings, catalog picker with conflict indicators, session-level overrides *(unverified what is already covered)*.

## ACP protocol
- [ ] **Session lifecycle**: only `session/load` is used. `session/list`, `session/resume`, `session/close` and `session/fork` are not implemented.
- [ ] **Plan improvements**: priority badges exist; plan persistence and historical lookup are open *(unverified)*.
- [ ] **B6 prewarm for claude-native/codex/pi**: deferred by measurement (about 0.8 s gain); see B6 in [../reviews/review-2026-10.md](../reviews/review-2026-10.md).
- [ ] **Client registry PR**: submit Jockey to the ACP client list.

## Web companion
- [ ] `/model`, `/mode` and `/effort` are desktop-only composer UI commands. Move them into the engine so the web companion can use them.

## UI polish
- [ ] **Reduce legacy `theme-*` classes**: still used in 10+ components (`App.tsx`, `ToolCallGroup.tsx`, `PermissionModal.tsx`, `SettingsPage.tsx`, `TerminalPanel.tsx`, ...) and `index.css`.
- [ ] **Searchable select primitive** for branch pickers *(unverified)*.
- [ ] **Token cleanup**: collapse duplicate token blocks in `index.css` *(unverified)*.
- [ ] **Desktop polish**: permission cache TTL, skeleton loaders, background throttling config, high-risk command confirms, aesthetics (frosted glass, floating command bars) *(unverified)*.
