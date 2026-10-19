# Engine service layer

Status: P1–P3 implemented (`src-tauri/src/engine/`). Supersedes the ad-hoc persistence that used to live in the desktop
frontend and the duplicate logic in `web_gateway/api.rs`. Post-implementation fixes are tracked in
[docs/reviews/review-2026-10.md](../reviews/review-2026-10.md).

## Problem

Jockey has three clients of the same conversation state — the desktop Solid UI, the remote
web companion, and agents talking to the Jockey MCP bridge — but only the desktop UI holds
the logic that makes a conversation durable. The Rust side runs agents and stores rows; the
desktop frontend decides what a turn *is*.

The table below is the original (pre-engine) state, now resolved; the file and function names no longer describe the code.

| Concern | Lived in (pre-engine) | Consequence |
| --- | --- | --- |
| Turn transcript (text / thought / tool segments, tool upserts, thought cap) | `src/lib/acpEventBridge.ts`, `useStreamEngine.finalizeSessionStream` | Any non-desktop client must re-implement it; the companion persisted text only. |
| Persisting the user message and the reply, streaming checkpoints | `useSessionManager.persistMessage` / `checkpointStreamingMessage` | A turn sent while the desktop window is closed (or from the web) is not recorded the same way. |
| Turn lifecycle (started / finished / cancelled) | Implicit in the `invoke("assistant_chat")` promise on the desktop | No event exists; other clients guess completion from stream silence. |
| Busy guard, cancel-and-drain, queue dequeue + inbox claim/restore, steer | `useMessageSend`, `useAgentContext.cancelCurrentRun` | Web cannot queue; two clients can start overlapping turns in one session. |
| `@role` routing and validation | `src/lib/chatPipeline.ts` **and** `src-tauri/src/parser.rs` | Two parsers; the frontend one also mutates session state (active role / runtime). |
| Auto-title from first message | `sessionHelpers.deriveSessionTitleFromMessage` | Companion-created sessions stay `Session_1`. |
| Runtime choice for a new session | `App.preferredAssistantForProject` **and** `web_gateway::api::preferred_runtime` | Already forked once. |
| Change notification for projects / sessions | none; `companion/changed` was added ad hoc | Each client polls or reloads differently. |

## Target shape

```text
┌──────────── views (render + collect input; no business rules) ────────────┐
│  Desktop Solid UI        Web companion (HTML)        Jockey MCP handlers   │
└───────▲────────────────────────▲──────────────────────────▲────────────────┘
        │ Tauri invoke / events  │ HTTP + SSE               │ MCP tools
┌───────┴────────────────────────┴──────────────────────────┴────────────────┐
│ adapters: commands/*  ·  web_gateway/api.rs  ·  jockey_mcp/handlers/*      │
│           thin: decode request → call service → encode result             │
└───────────────────────────────────┬─────────────────────────────────────────┘
┌───────────────────────────────────┴─────────────────────────────────────────┐
│ engine (src-tauri/src/engine/)                                              │
│   turns       submit / cancel / steer, busy registry, queue drain, routing  │
│   transcript  TurnRecorder: AcpEvent stream → AppMessage, checkpoints       │
│   sessions    create / reopen / title / runtime preference                  │
│   events      typed engine events, one emitter for every client             │
├─────────────────────────────────────────────────────────────────────────────┤
│ existing: chat core (`chat.rs`, `chat/session_runtime.rs`) · acp worker/adapters · db/*            │
└─────────────────────────────────────────────────────────────────────────────┘
```

Rules:

1. **The engine owns every durable fact.** A message row, a session title, a queue item, a
   turn's state — written by the engine, never by a view. Views may hold optimistic copies
   keyed by ids the engine accepts (`clientMessageId`), and replace them with what the
   engine returns.
2. **One event stream, consumed identically.** `acp/delta` and `acp/stream` stay the live
   preview channel. Lifecycle and change facts go out as engine events (below) that the
   desktop listens to over Tauri and the web over SSE — same names, same payloads.
3. **Adapters are thin.** No adapter decides runtime, role, persistence, or busy state. If a
   second adapter needs a rule, the rule moves into the engine.
4. **Live rendering is a projection, not a record.** The desktop reducer
   (`acpEventBridge`) keeps rendering the in-flight turn for latency; when `turn/finished`
   arrives, the persisted messages it carries replace the live projection.

### Engine events

| Event | Payload | Emitted when |
| --- | --- | --- |
| `turn/started` | `{ appSessionId, turnId, roleLabel, userMessage }` | A turn is accepted and its user message persisted. |
| `turn/finished` | `{ appSessionId, turnId, ok, cancelled, error?, messages: AppMessage[] }` | The turn ended (success, failure, or cancel) and its transcript is persisted. |
| `queue/changed` | `{ appSessionId, items: InboxMessage[] }` | The session's queued input changed; views replace their copy with `items`. |
| `message/added` | `{ appSessionId, message: AppMessage }` | A message was persisted outside a turn transcript (steered input, control-command echo, entries imported from a provider transcript by `engine::watch`). |
| `turn/progress` | `{ appSessionId, turnId, rev }` | The running turn moved, once a second at most, only for watched sessions; late joiners pull `get_live_turn` (see session-sync.md). |
| `session/changed` | `{ appSessionId, projectId, reason, session: SessionSummary }` | Created, updated, closed, reopened, imported. |
| `project/changed` | `{ projectId, reason }` | Created, deleted. |

`turnId` is the engine's id; the persisted assistant message for a single-role turn uses it as
`client_id`, so streaming checkpoints and the final write are one row (upsert), never two.

## Phases

Each phase leaves the desktop fully working and is committed separately.

**P1 — transcript and lifecycle in the engine** (fixes the two companion bugs) — done
- `engine::transcript::TurnRecorder` ports the reducer that matters for the record
  (text, thought + 5000-char thought cap, toolCall / toolCallUpdate / toolOutputDelta upserts,
  unknown → `other` segments, newline normalisation, tool sanitising) and the finalize rules
  (final reply replaces the trailing text segment; per-role split for multi-agent turns;
  partial content kept on failure/cancel).
- `submit_input` (`engine::service::submit`) persists the user message (`clientMessageId` from the caller), records the
  turn, checkpoints every 1.5 s, persists the result, emits `turn/started` / `turn/finished`,
  and returns the persisted messages.
- Desktop stops persisting turn messages and adopts the returned ones; the web drops its own
  persistence and the "4 s of silence" heuristic.

**P2 — turn service** — done
- Busy registry and cancel live in the engine (`cancel_turn`); queued input drains from the
  inbox in the engine after `turn/finished`; `@role` routing has one implementation (Rust)
  that also returns the role/runtime the session switched to; auto-title moves to the engine.
- `useMessageSend` shrinks to: optimistic user bubble → `submit_input` → render events.
- Web gains queueing and cancel with no extra code.
- As built: `engine::service::{submit, cancel, send_queued_now}` behind `submit_input`,
  `cancel_turn`, `send_queued_input_now`, `remove_queued_input` (Tauri) and the gateway's
  `/chat`, `/cancel`, `/queue`, `/send-now`. `turns::reserve` makes the busy check atomic.
  Every view opens/settles live turns from `turn/started` / `turn/finished` (desktop:
  `src/lib/engineEvents.ts`), syncs queues from `queue/changed`, and steered/control input
  from `message/added`. Multi-agent turns no longer run every mentioned persona on the
  session's selected engine.
- The earlier adapter issue (empty result after cancelling a `claude-native` turn) is fixed; see review B1/B5.
- Gateway routes (`web_gateway/api.rs`, all behind the token): `/status`, `/projects` (+ `/:id/importable`, `/:id/import`), `/sessions` (+ `/closed`, `/:id`), `/sessions/:id/{chat,cancel,queue,queue/:item/remove,send-now,reopen,close,role}`, `/roles`, `/fs/dirs`, `/permissions` (+ `/respond`), and `/events?focus=<sid>` (SSE).
- The web role picker calls `POST /sessions/:id/role`, which goes through `engine::sessions::select_role`; persona editing stays desktop-only.

**P3 — session service, change events, web read models** — done
- Runtime preference, draft-session persistence and reopen/close go through
  `engine::sessions`; `session/changed` / `project/changed` replace `companion/changed`.
- `web_gateway/api.rs` loses `preferred_runtime`, its running map and its own persistence;
  the desktop listener is `src/lib/engineEvents.ts`.
- Web: open a project by browsing host folders (not typing paths), close sessions, import
  CLI transcripts. Deliberately left out of the web: deleting projects, persona/role editing,
  MCP/skills/settings — destructive or desktop-shaped, rarely needed from a phone.

## P3 design decisions

Each decision lists the options considered and why the chosen one won.

### 1. Who emits change events (observer placement)

| Option | Problem |
| --- | --- |
| Each adapter emits after calling the DB (Tauri cmd, gateway, MCP handler) | Three copies; the MCP handlers have no `AppHandle`, so their changes would stay invisible. |
| The DB layer emits | Repositories would depend on Tauri, and every DB unit test would need an app. |
| **Engine ops wrap the repository and emit through one hub** (`engine::events`) | — |

Chosen: domain operations in `engine::sessions` (create / close / reopen / update / import,
project create / delete) are the only writers; adapters call them. `engine::events` holds the
`AppHandle` installed at startup, so callers without one (MCP handlers) still notify, and it
is a no-op in unit tests. This is a Facade (`engine::service`, `engine::sessions`) in front of
the repositories (`db::*`), with Observer-style fan-out through a single emitter.

### 2. Event payload shape

| Option | Cost |
| --- | --- |
| Thin (`{ id }`), clients refetch | Every connected client refetches; the session list endpoint loaded every message of 100 sessions just to build previews. |
| Fat (full session with messages) | Sends whole transcripts to every client on every rename. |
| **Delta with a summary row** | — |

Chosen: `session/changed { appSessionId, projectId, reason, session: SessionSummary }`. Views
patch their list in place and fetch a transcript only when opening it. Turn content is the
one deliberate exception — `turn/finished` carries the turn's messages, because every view
needs exactly those to replace its live preview, and refetching would double the traffic.

### 3. Read models (CQRS-lite)

Commands mutate through the engine; queries read purpose-built projections:
`engine::sessions::summaries` computes message count and last preview with indexed
subqueries (`app_session_messages(session_id, id)`) instead of materialising every message,
and the web's transcript endpoint takes `?tail=N` so opening a long session on a phone does
not ship the whole history.

### 4. Subscription and fan-out to web clients

| Option | Trade-off |
| --- | --- |
| Broadcast everything to every SSE client | Simple, but `acp/delta` / `acp/stream` for every running session reach phones that show one. |
| Client-driven topic subscribe (connection id + POST) | Precise, but server-side connection state and an extra round trip. |
| **Per-connection focus filter (`/api/events?focus=<sid>`)** | Reconnect on session switch (cheap); the reload that a switch does anyway covers the gap. |

Chosen: lifecycle events (`turn/*`, `queue/changed`, `session/changed`, `project/changed`,
permissions) go to every client; high-frequency stream frames only to clients focused on that
session. The broadcast buffer is sized for bursts, and a client that still lags receives a
`resync` event and reloads instead of silently missing frames.

### 5. Concurrency

- **Per session: serial.** One reservation per session; a session loop runs a turn, then
  drains the queue in order. This is the actor pattern without a mailbox type — the inbox
  table is the mailbox, so queued input survives restarts.
- **Across sessions: parallel.** Each session loop is its own task; nothing global serialises
  turns.
- **Within a multi-agent turn: serial (kept).** Running `@A @B` in parallel would be faster,
  but later personas receive earlier personas' replies as context and they share one working
  tree; parallel runs would race on files and lose the hand-off.
- **Blocking work off the async runtime.** SQLite, transcript scans, folder listing and the
  agent catalog run in `spawn_blocking`; the event tap holds its lock only for an in-memory
  update (see `turns::route`).

### 6. Opening a project from the web

| Option | Verdict |
| --- | --- |
| Type an absolute path | Kept as a fallback; painful on a phone. |
| Discover project folders from CLI transcript history | Needs a full transcript scan across every provider; slow and lossy (Claude encodes paths in directory names). |
| **Browse host folders, starting from where existing projects live** | Directories only, dotfiles hidden, git repos and already-added projects marked. |

Chosen: `GET /api/fs/dirs?path=` lists sub-directories (bounded, blocking pool). It opens at
the most common parent of existing projects (e.g. `~/Documents/GitHub`), so the usual case is
one tap. It exposes no more than the token already grants — the same token lets agents run
shell commands on the host.

## Testing

- Rust unit tests for `TurnRecorder` (segment ordering, tool upsert, final-reply replacement,
  per-role split, thought cap, checkpoint/final upsert to one row).
- No frontend test harness exists; each phase lists manual checks in its commit/PR: single
  turn, multi-agent `@A @B`, cancel mid-tool, queued message after a turn, reload after each.
