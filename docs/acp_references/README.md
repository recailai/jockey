# ACP Reference Implementations

This folder is a **research index** for Jockey's ACP (Agent Client Protocol) integration. It does **not** document Jockey's own code — it documents what the top-2 ACP clients in the ecosystem do, so we have a shared vocabulary and a concrete checklist of proven patterns to borrow from when we work on `src-tauri/src/acp/`.

## Why this folder exists

Jockey's ACP layer has grown organically. When a new bug lands (connection death, prompt timeout, permission hang, sequence gap) we usually end up re-deriving patterns that Zed and AionUi solved years ago. This folder captures those patterns once, with file:line pointers into the upstream code, so we can read instead of re-derive.

## The two reference projects

| Rank | Project | Stars | Stack | Why we care | Canonical file |
|---|---|---|---|---|---|
| 1 | [zed-industries/zed](https://github.com/zed-industries/zed) | 79.6k | Rust + GPUI | ACP originator; same-language SDK user; canonical Rust integration | [`crates/agent_servers/src/acp.rs`](https://github.com/zed-industries/zed/blob/main/crates/agent_servers/src/acp.rs) (~3,400 lines) |
| 2 | [iOfficeAI/AionUi](https://github.com/iOfficeAI/AionUi) | 22.5k | TypeScript + Electron | Closest in scope to Jockey (multi-agent desktop orchestrator); explicitly documented refactor using Zed + acpx as references | [`src/process/acp/`](https://github.com/iOfficeAI/AionUi/tree/main/src/process/acp) (multi-file, well-layered) |

See [`zed.md`](./zed.md) and [`aionui.md`](./aionui.md) for per-project deep-dives, and [`best_practices.md`](./best_practices.md) for the prioritized checklist of patterns Jockey could adopt.

## How Jockey's modules map to reference concepts

This is the index: when you're about to touch a Jockey file, these are the reference-project sections that solve the same problem.

| Jockey file | Concept | Zed analogue | AionUi analogue |
|---|---|---|---|
| `acp/adapter.rs` | runtime_key → binary/args resolution | `agent_servers.rs` (`AgentServer` trait), `custom.rs` | `process/agent/acp/AcpDetector.ts` |
| `acp/client.rs` | `impl Client for ...` (readTextFile, permission, terminals) | `acp.rs:3032-3350` (`handle_request_permission`, `handle_read_text_file`, `handle_create_terminal`) | `ProcessAcpClient.ts:start()` Client builder + `session/PermissionResolver.ts` |
| `acp/runtime_state.rs` | cached discovered modes/config options | `AcpSession.session_modes` + `config_options` (`Rc<RefCell<...>>`) | `session/ConfigTracker.ts` (dual-state desired/current) |
| `acp/session/cold_start.rs` | new_session handshake + apply defaults | `acp.rs:1106-1270` (`new_session`, optimistic mode update) | `session/SessionLifecycle.ts:doStart()` |
| `acp/session/execute.rs` | high-level prompt wrappers | `AcpThread::send` chain | `session/PromptExecutor.ts` |
| `acp/session/mcp.rs` | MCP server list builder | `acp.rs:2783-2830` (`mcp_servers_for_project`) | `session/McpConfig.ts` |
| `acp/session/prewarm.rs` | pre-create connections for fast first prompt | (none — Zed creates lazily on first send) | (none — AionUi creates on first message) |
| `acp/session/session_cmds.rs` | cancel / reset / reconnect / set_mode | `acp.rs:1368` (`close_session`), `acp.rs:1150` (SetSessionMode) | `session/SessionLifecycle.ts:cancel/resume` + `ConfigTracker.ts:setDesiredMode` |
| `acp/worker/handlers.rs` | `handle_execute`, `handle_prewarm` | (inline in `acp.rs`, no equivalent god-function) | `session/PromptExecutor.ts` (single-responsibility per step) |
| `acp/worker/mod.rs` | `WorkerMsg` dispatch loop | dispatch-queue pattern (`ForegroundWork` mpsc) | (n/a — TS is single-threaded, no bridge needed) |
| `acp/worker/notify.rs` | death / prewarm event senders | `emit_load_error_to_all_sessions` (`acp.rs:1019`) | `ProcessAcpClient.onDisconnect` |
| `acp/worker/permission.rs` | permission request routing | `handle_request_permission` (`acp.rs:3032`) | `session/PermissionResolver.ts` (full cascade) |
| `acp/worker/pool.rs` | `CONN_MAP`, `LiveConnection`, cancel handles | `sessions: Rc<RefCell<HashMap<SessionId, AcpSession>>>` | `runtime/AcpRuntime.ts` sessions map + `IdleReclaimer` |
| `acp/worker/types.rs` | `WorkerMsg`, events | `ForegroundWork` enum | (n/a) |

## Reading order for newcomers

1. **Start with the non-obvious threading pattern**: [`zed.md § 2 — Handler dispatch + `into_foreground_future`](./zed.md#2-handler-dispatch--into_foreground_future). This is the pattern that bridges `Send` SDK handlers to a `!Send` UI thread — exactly Jockey's situation (worker thread + Tauri frontend). 15-line helper, explains why `block_task` deadlocks.

2. **Then read the layering that Jockey is missing**: [`aionui.md § 1 — Single Owner `ProcessAcpClient`](./aionui.md#1-single-owner-processacpclient). One class owns process + connection + lifecycle. Jockey's equivalents are scattered across `worker/pool.rs` + `worker/handlers.rs` + `worker/notify.rs`.

3. **Then the error layer**: [`aionui.md § 6`](./aionui.md#6-error-handling). AionUi built what Jockey most conspicuously lacks: a canonical error enum with retryability flags, a recursive payload extractor, and a JSON-RPC-code-to-app-code map. Jockey currently passes `String` everywhere.

4. Finally, the [`best_practices.md`](./best_practices.md) checklist for a prioritized view of "what's worth adopting, and what's already in place."

## Related existing docs

- **[`../acp_sdk_notes.md`](../acp_sdk_notes.md)** — Jockey-specific SDK gotchas (non-exhaustive structs, builder methods, v0.10.2/v0.11.2 quirks). Not duplicated here.
- **[`../rfd_mcp_over_acp.mdx`](../rfd_mcp_over_acp.mdx)** — RFD for MCP-over-ACP channel design.
- **[`../rfd_session_fork.mdx`](../rfd_session_fork.mdx)** — RFD for forked sessions (PR summaries etc.).
- **[`../rfd_meta_propagation.mdx`](../rfd_meta_propagation.mdx)** — RFD for W3C trace context in `_meta`.
- **[`../rfd_proxy_chains.mdx`](../rfd_proxy_chains.mdx)** — RFD for middleware chains over ACP.

## Scope & maintenance

These docs are **static snapshots**. Zed and AionUi evolve; line numbers and even file paths will drift. When referring to a pattern here, always re-verify the upstream source. If a pattern here is adopted into Jockey, add a `→ see <Jockey-file>` note to the corresponding row in `best_practices.md` rather than deleting the reference (we still want to know where the pattern came from).
