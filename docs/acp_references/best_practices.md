# ACP Best Practices Checklist

Prioritized patterns from [Zed](./zed.md) and [AionUi](./aionui.md) that could land in Jockey's `src-tauri/src/acp/`. One row per pattern. Columns:

- **Source** — which reference project the pattern comes from (Zed / AionUi / both)
- **Jockey status** — `✅ have`, `⚠️ partial`, `❌ missing`
- **Where in Jockey** — the file that has or would host the pattern
- **Effort** — rough size if we wanted to adopt (`S` ≤ 1 day, `M` 2–4 days, `L` 1+ week)
- **Priority** — `H` / `M` / `L` based on value-for-effort for Jockey's current state

The table is meant to be scannable. Click through to `zed.md` / `aionui.md` for the how/why of each pattern.

---

## Transport & lifecycle

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Backpressured outgoing sink (write `.await`s, SDK yields if pipe full) | Zed (`acp.rs:588-603`) | ✅ have | `worker/pool.rs` + commit 44dc5dd ("apply backpressure on stream channel") | — | — |
| 4-signal death detection (`exit` + `close` + `stdout.close` + `connection.signal.abort`) | AionUi (`ProcessAcpClient.ts:355-366`) | ⚠️ partial — only `child.status` + `health_rx` | `worker/pool.rs`, `worker/notify.rs` | S | **H** — misses pipe-only EOF on gemini/codex |
| `Promise.race(initialize, startupFailureWatcher)` with stderr at reject-time | AionUi (`ProcessAcpClient.ts:135-156`) | ❌ missing | `session/cold_start.rs` | S–M | **H** — turns "connection disappeared" noise into real stderr |
| Stderr ring buffer (8KB, captured from spawn) for failure context | AionUi (`ProcessAcpClient.ts:61-75`) | ⚠️ partial — stderr forwarded to `acp_log`, not buffered | `worker/pool.rs` | S | M |
| `wait_task` broadcasts `LoadError::Exited` to all sessions of the connection | Zed (`acp.rs:650-660`, `acp.rs:1019`) | ⚠️ partial — per-connection `health_rx::changed()` polling | `worker/notify.rs` | M | M |

## Handler dispatch

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Single-threaded runtime for `!Send` `ClientSideConnection` (no cross-thread work) | Zed: dispatch queue / AionUi: n/a (JS single-threaded) | ✅ have — `LocalSet` on dedicated thread | `worker/mod.rs` | — | — |
| Use `on_receiving_result` + oneshot inside handler callbacks, not `block_task` (avoids dispatch-loop deadlock) | Zed (`acp.rs:58-73`) | ⚠️ audit needed — most handlers don't `.await` nested requests, but `permission.rs` does | `worker/permission.rs`, `client.rs` | S | M — audit + document the rule |
| Typed error downcast preserving `AuthRequired` through `anyhow` | Zed (`acp.rs:1626-1638`) | ❌ missing — `String` errors everywhere | `acp/error.rs` (new) | M | **H** (see errors row below) |

## Session management

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Single-owner session object (state + queue + resolvers in one place) | AionUi (`AcpSession`) | ❌ missing — state split across SQLite + `CONN_MAP` + `runtime_state` | N/A (architectural) | L | L — not worth it until we hit a concrete bug |
| Ref-counted session map (survives pending-load racing close) | Zed (`acp.rs:278-285` + `close_session`) | ❌ missing | `worker/pool.rs` | M | L — Jockey's narrower race is held by `PROMPT_LOCKS` |
| Fork / load / resume via native ACP methods | Zed (`load_session`, `resume_session`) | ⚠️ partial — `resume_session_id` on prewarm + `reset/reconnect` commands; no `fork` | `session/cold_start.rs`, `session_cmds.rs` | M | M — blocked on `docs/rfd_session_fork.mdx` |
| Dual-state `ConfigTracker` (desired vs. current; reconcile before prompt) | AionUi (`ConfigTracker.ts`) | ❌ missing — `set_acp_mode` is request-then-persist | `runtime_state.rs`, `session/session_cmds.rs` | M | L — revisit after idle reclaim |
| Optimistic mode update (mutate local state, roll back on RPC failure) | Zed (`acp.rs:1150-1270`) | ❌ missing | `session/session_cmds.rs` | S | L — UX polish only |

## Prompt execution

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Per-session prompt serialization (one in flight at a time) | AionUi (`PromptQueue` + `drainLoop`), Zed (implicit) | ✅ have via `PROMPT_LOCKS` | `worker/pool.rs` | — | — |
| Expose queue depth to UI ("your prompt is N in line") | AionUi (arch doc — explicit queue) | ❌ missing — `PROMPT_LOCKS` is a mutex | `worker/pool.rs`, `types.rs` | S | L — UX polish |
| Pause prompt timer during permission wait | AionUi (`PromptTimer.pause/resume`) | ❌ missing — `PROMPT_LIVENESS_INTERVAL` ticks during waits | `worker/handlers.rs` | S | M — affects perceived timeouts |
| Cancel notification + drain (send `CancelNotification`, wait for `StopReason::Cancelled`) | Zed + AionUi | ✅ have — `CANCEL_HANDLES` + 5s bounded wait on `PROMPT_LOCKS` | `worker/mod.rs` `WorkerMsg::Cancel` arm | — | — |

## Permissions

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Pending-request map keyed by `callId` / `request_id` | AionUi, Zed | ✅ have | `worker/permission.rs` | — | — |
| YOLO mode (auto-approve all permissions) | AionUi (`yoloMode` flag) | ❌ missing | `worker/permission.rs` | S | M — requested feature per general multi-agent UX |
| LRU cache of "allow_always" decisions (never cache denials) | AionUi (`ApprovalCache`) | ❌ missing | `worker/permission.rs` | S | **H** — low-risk UX win |
| Reject all pending permissions on session crash | AionUi (`cancelAll()`) | ⚠️ partial — permission waits don't auto-clear on connection death | `worker/permission.rs`, `worker/notify.rs` | S | M |

## Errors

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Typed error enum (`AcpError { Auth, Retryable, Fatal, Cancelled, ... }`) replacing ad-hoc strings | AionUi (`AcpError.ts`), Zed (`AuthRequired` + `LoadError`) | ❌ missing | `acp/error.rs` (new) | M | **H — single highest-value pattern for Jockey** |
| Recursive `extractAcpError` walking `error`/`cause`/`acp` up to 5 levels deep | AionUi (`errorExtract.ts:15-37`) | ❌ missing | `acp/error.rs` | S | H (pairs with row above) |
| Errno / kind → retryable mapping (broken pipe, connection reset, timeout) | AionUi (`errorNormalize.ts`) | ❌ missing | `acp/error.rs` | S | H (pairs with row above) |
| `AuthRequired` downcast through `anyhow::Error` preserving typed error | Zed (`acp.rs:86-108, 1626-1638`) | ❌ missing | `acp/error.rs` | S | M |
| `LoadError` variants for unrecoverable failures (install fail / exit / version mismatch) | Zed (`acp_thread.rs:1171-1200`) | ⚠️ partial — we emit structured `acp_log` stages but no enum | `acp/error.rs` | S | M |

## Observability

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Structured stage logs for every ACP phase (`stage.ok`, `pool.invalidate`, ...) | Zed + Jockey's own `acp_log` | ✅ have | `acp/adapter.rs::acp_log` | — | — |
| Opt-in per-connection log tap for in-app debug panel | Zed (`AcpConnectionRegistry`, `acp.rs:565-580`) | ❌ missing | would live in new `acp/log_tap.rs` | M | L — depends on whether we build the panel |
| Metrics interface (spawn / init / first-token latency, error counts, resume success) | AionUi (`AcpMetrics.ts`) | ❌ missing — only log lines today | `acp/metrics.rs` (new) | S | M — thin trait + no-op default is cheap |
| Sequence-gap diagnostics on streaming channels | Jockey-only (commit 44dc5dd) | ✅ have | `worker/handlers.rs`, `worker/types.rs` | — | — |

## Pool / runtime

| Pattern | Source | Jockey status | Where in Jockey | Effort | Priority |
|---|---|---|---|---|---|
| Connection pool keyed by `(session_id, runtime, role)` | Jockey-only (not upstream pattern) | ✅ have | `worker/pool.rs::CONN_MAP` | — | — |
| Time-based idle reclaim (5-minute default, suspend not kill) | AionUi (`IdleReclaimer.ts`) | ❌ missing — evicts only on cwd change or error | `worker/pool.rs` | M | M — needs resume-via-loadSession workflow |
| Session-scoped credentials (per-negotiator, not global env) | AionUi (`AuthNegotiator`) | ❌ missing — Jockey inherits ambient env | `adapter.rs` | L | L — users are devs with CLIs pre-configured |

---

## Recommended next steps, in order

Three rows from the table above stand out as **high priority / low-medium effort** and compose cleanly:

1. **`acp/error.rs`** — the typed `AcpError` enum + `extract_acp_error` + `normalize_error`. Touches every `.map_err(|e| e.to_string())` call site, but the rewrite is mechanical. Unblocks everything else (cache-by-error-code, retry logic, auth detection). Prior art: both reference projects.
2. **Startup race in cold-start** — `tokio::select!` between `initialize` and `child.wait()`, include last 8KB of stderr in the error message. 1-file change in `session/cold_start.rs`, huge UX win on first-run failures.
3. **4-signal death detection** — add `stdout.close` and `connection.signal.abort` (or their Rust equivalents via `acp::ClientSideConnection`'s shutdown channel) to `worker/pool.rs::LiveConnection`. Catches pipe-only EOF cases that currently hang until the liveness interval trips.

These three together cover 80% of "connection died, frontend has no idea why" reports. The rest of the rows (idle reclaim, log tap, YOLO mode, dual-state config) are independent and can land any time.

**Not recommended right now**: single-owner session object, ref-counted session map. These are refactors, not features — cost is high and Jockey's structure disagrees with AionUi's 1:1 conversation model. Revisit if/when the session/workflow model simplifies.

---

## Writing RFDs from this table

When a pattern here graduates from "interesting" to "we're building it", the row becomes the seed for an RFD under `docs/rfd_*.mdx`. Suggested naming:

- Row "typed `AcpError` enum" → `docs/rfd_acp_error_layer.mdx`
- Row "4-signal death detection" → `docs/rfd_connection_death_signals.mdx`
- Row "Time-based idle reclaim" → `docs/rfd_idle_reclaim.mdx`

Each RFD should cite the specific row(s) here and the upstream file:line from [`zed.md`](./zed.md) / [`aionui.md`](./aionui.md). That way future readers can verify the pattern in-situ even as line numbers drift.
