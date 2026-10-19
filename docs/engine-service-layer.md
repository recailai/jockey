# Engine service layer

Status: P1 and P2 implemented (`src-tauri/src/engine/`); P3 pending. Supersedes the ad-hoc persistence in `useMessageSend` / `useSessionManager`
and the duplicate logic in `web_gateway/api.rs`.

## Problem

Jockey has three clients of the same conversation state — the desktop Solid UI, the remote
web companion, and agents talking to the Jockey MCP bridge — but only the desktop UI holds
the logic that makes a conversation durable. The Rust side runs agents and stores rows; the
desktop frontend decides what a turn *is*.

| Concern | Lives in today | Consequence |
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
│ existing: chat::assistant_chat core · acp worker/adapters · db/*            │
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
| `session/changed` | `{ appSessionId, projectId?, reason }` | Created, reopened, closed, renamed, imported. |
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
- `assistant_chat` persists the user message (`clientMessageId` from the caller), records the
  turn, checkpoints every 1.5 s, persists the result, emits `turn/started` / `turn/finished`,
  and returns the persisted messages.
- Desktop stops persisting turn messages and adopts the returned ones; the web drops its own
  persistence and the "4 s of silence" heuristic.

**P2 — turn service** — done
- Busy registry and cancel live in the engine (`cancel_turn`); queued input drains from the
  inbox in the engine after `turn/finished`; `@role` routing has one implementation (Rust)
  that also returns the role/runtime the session switched to; auto-title moves to the engine.
- `useMessageSend` shrinks to: optimistic user bubble → `submit_turn` → render events.
- Web gains queueing and cancel with no extra code.
- As built: `engine::service::{submit, cancel, send_queued_now}` behind `submit_input`,
  `cancel_turn`, `send_queued_input_now`, `remove_queued_input` (Tauri) and the gateway's
  `/chat`, `/cancel`, `/queue`, `/send-now`. `turns::reserve` makes the busy check atomic.
  Every view opens/settles live turns from `turn/started` / `turn/finished` (desktop:
  `src/lib/engineEvents.ts`), syncs queues from `queue/changed`, and steered/control input
  from `message/added`. Multi-agent turns no longer run every mentioned persona on the
  session's selected engine.
- Known adapter issue (not engine): after cancelling a `claude-native` turn, the next prompt
  on the same provider session can return an empty result immediately.

**P3 — session service and change events**
- Runtime preference, draft-session persistence and reopen/close go through
  `engine::sessions`; `session/changed` / `project/changed` replace `companion/changed`.
- `web_gateway/api.rs` loses `preferred_runtime`, its running map and its own persistence;
  `src/lib/companionSync.ts` becomes a generic `session/changed` listener.

## Testing

- Rust unit tests for `TurnRecorder` (segment ordering, tool upsert, final-reply replacement,
  per-role split, thought cap, checkpoint/final upsert to one row).
- No frontend test harness exists; each phase lists manual checks in its commit/PR: single
  turn, multi-agent `@A @B`, cancel mid-tool, queued message after a turn, reload after each.
