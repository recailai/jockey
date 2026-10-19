# Session sync: late joiners and external CLI activity

Status: read-side sync implemented (see [As built](#as-built)); the adapter-contract
extensions below (Codex shared daemon, sequenced frames) remain design. Builds on
[engine-service-layer.md](engine-service-layer.md) and the adapter contract in
[runtime-adapters.md](runtime-adapters.md).

## As built

The requirement is read-only following with eventual consistency, plus a running state. That
admits a much smaller mechanism than the full model below, so the implementation picks the
cheapest option that is exact at the end of every turn:

| Concern | Options weighed | Chosen |
| --- | --- | --- |
| View joins a running Jockey turn | (a) sequence every frame (`rev`) through an engine publisher, clients dedupe; (b) clients poll the checkpoint; (c) **notify, then pull**: a tiny `turn/progress { turnId, rev }` while someone watches, late joiners fetch the in-memory snapshot | (c): no change to the ~8 adapter emit sites or to the desktop's live path, ≤ 1 s staleness, and `turn/finished` already makes the record exact. (a) stays the upgrade path if sub-second fidelity for late joiners is ever needed. |
| Provider conversation driven elsewhere | (a) join the vendor owner (Codex daemon); (b) OS file watcher (`notify`); (c) **stat the transcripts of watched sessions once a second** and import appended entries through the importer | (c): no new dependency, works for Claude, Codex and Pi alike, costs one `stat` per watched conversation per second. (a) gives live frames for Codex but changes how turns run; (b) saves a second of latency at the cost of a dependency. |
| Running state | (a) per-adapter process detection; (b) **transcript growth with no Jockey turn running** | (b): `externalActive` in session summaries, true for 8 s after the transcript last grew. |

Pieces:

- `engine::watch` — refcounted `watch` / `unwatch` per session (desktop: the shown session;
  web: the SSE `focus`, released when the connection drops). One loop per watched session,
  stopped when the count reaches zero; nothing runs for unwatched sessions.
- `turns::{live_rev, live_turn}` — the running turn's revision and snapshot, read under the
  turn lock; `get_live_turn` (Tauri) and `GET /api/sessions/:id/live` (web) expose it, and
  `watch_session` returns it so a view opened mid-turn renders immediately.
- Transcript cursor — `app_session_role_runtime_configs.transcript_cursor` counts the
  provider transcript entries already reflected in the session. `importer::sync_transcript`
  adds only entries past it, under the importer's deterministic ids, so importing and
  watching share one idempotent path. A conversation without a cursor starts after its
  imported rows if its newest message is one of them, otherwise at the transcript's end. A new
  provider id resets the cursor.
- Own-write invariant — `turns::begin` marks the session as being written by Jockey (the
  watcher skips it); 2 s after the turn ends `importer::mark_transcript_read` moves the cursor
  to the transcript's end, so neither the watcher nor a re-import records Jockey's own turn
  twice. This also fixes re-import duplicating turns Jockey ran on an imported session.
- After importing external entries the provider slot is discarded, so the next Jockey turn
  resumes from the provider's record instead of a warm process that missed those turns.
- `submit` refuses input while `externalActive` (resuming a conversation another process is
  mid-turn in interleaves both, per the Claude Code docs).
- Views: desktop `engineEvents.ts` (`turn/progress` → `adoptLiveTurn`, ignoring turns it saw
  finish) and a spinner titled "Running outside Jockey" on the session chip; web
  `companion.html` renders snapshots instead of raw frames when it did not see the turn start,
  and shows 终端运行中.

Verified end to end through the gateway (claude-native): a client subscribing 4 s into a
turn received `turn/progress` and the snapshot, then the persisted record; a terminal
`claude -p --resume <id>` run while watched produced `externalActive` within a second, both
of its messages as `message/added`, a refused submit, and `externalActive` false 8 s after
it ended; the next Jockey turn answered from the terminal's turn (slot discarded), and its
own entries were not re-imported.

Not covered: imported messages are text-only (the importer's projection: no tool cards or
thoughts); a conversation driven elsewhere *before* Jockey first watched it is only picked up
if its newest stored message came from the transcript.

## Requirements

1. A view that opens a session **while a turn is running** (desktop window reloaded, project
   switched in, phone opening the web companion) shows the turn's progress so far and then
   follows it live — no gap, no duplicated text — regardless of which client started it.
2. When the session's **provider conversation is being driven outside Jockey** (the user runs
   `claude --resume <id>` / `codex resume` in a terminal), Jockey can follow it.
3. **Watching never takes ownership.** Opening or viewing a session must not reserve it,
   spawn or warm an agent process, or block another client from sending.

## Where we are

| Case | Today | Gap |
| --- | --- | --- |
| Two Jockey clients, both open before the turn starts | `turn/started` → live frames → `turn/finished` reach every client (Tauri + SSE). | None. |
| Client opens a session mid-turn | Desktop: `engineEvents.ts` only accepts frames for turns it saw start (`acceptingStreams`), so a late window shows the last 1.5 s checkpoint row and nothing more until `turn/finished`. Web: loads the checkpoint, then appends raw deltas into a separate bubble. | Desktop freezes for the rest of the turn; web loses the frames between checkpoint and subscribe and shows the turn split in two. Frames carry no position, so neither side can dedupe or detect a gap. |
| Web SSE client lags | Gets `resync`, reloads the session. | Same mid-turn gap after the reload. |
| Provider conversation continues in a terminal | Nothing. A later re-import appends the new transcript entries (ids are deterministic). | No live view. Worse: Jockey's warm provider process still holds the *old* conversation in memory, and a Jockey turn sent now resumes in parallel with the terminal — the two diverge or interleave in one transcript file. |
| Viewing takes ownership | Already true that it does not: only `submit` takes the `turns::reserve` slot; config discovery uses a separate probe connection (`prewarm_role_config_cmd`). | Keep it that way. |

## How the agent CLIs handle it (checked 2026-10-19)

None of them supports two independent processes writing one conversation. The pattern they
converge on is **one owner process per conversation, other surfaces attach to it**.

| CLI | Two processes resume one session | What the vendor offers instead |
| --- | --- | --- |
| Claude Code 2.1.x | Documented as unsafe: "If you resume the same session in two terminals without forking, messages from both interleave into one transcript." | **Background sessions** (`claude --bg`, `claude agents`, `claude attach <id>`), hosted by a per-user supervisor: resuming a *running* background session attaches to that process instead of loading the transcript. A `claude -p --resume <id>` that sets output/config flags (as Jockey does) is refused with exit 1 and the `claude attach` hint. `--fork-session` / `/branch` copy the conversation. Remote Control lets a phone follow a session. The transcript JSONL is documented as an internal format that "changes between versions"; hooks receive `transcript_path` and are the supported trigger. |
| Codex 0.163 | Each `codex app-server` process loads the thread on its own. | A **shared local app-server daemon** (`codex app-server daemon start`, `codex agents`, `codex queue --thread`, TUI `--remote`). Clients of the same daemon subscribe to one in-memory thread and all receive `turn/*` notifications; `thread/resume` rebuilds state for a reconnecting client. `codex app-server proxy` bridges stdio to the daemon's control socket. `codex fork` / `thread/fork` branch a thread. (Multi-client semantics come from third-party write-ups; verify before relying on them.) |
| Pi 1.1 | Plain JSONL session files, no owner process. | `--fork <id>` into a new session; no daemon. |

Jockey's engine already is this pattern internally (one owner per app session, views
subscribe). The design below extends the same model one level down, to the provider
conversation, and expresses it in the existing adapter vocabulary so the engine stays
runtime-agnostic.

## Model

Four terms, used the same way for every runtime:

| Term | Meaning | Where it lives |
| --- | --- | --- |
| **Conversation** | The provider-side conversation behind an app session: runtime + provider handle (`resume_session_id`, the imported `external_session_id`) + an opaque locator (transcript file, thread id). | `ConversationRef`, built from the existing provider-handle rows |
| **Owner** | Who is driving the conversation right now: `None`, `Jockey { turn_id }`, `External { kind, since }` with `kind` = `Daemon` (vendor owner process Jockey also talks to), `Background` (vendor owner Jockey cannot join), `Unknown` (only observed activity). | Engine, in the session summary (`owner`) |
| **Observation** | A stream of the conversation's activity, normalised into the same frames adapters already emit (`AcpEvent` on `acp/delta` / `acp/stream`), plus a **cursor** — an opaque, per-runtime position (byte offset, last item id) that lets observation resume without re-reading. | Adapter produces it; engine consumes it |
| **Turn origin** | `Jockey` or `External`. An external turn is recorded and announced exactly like a Jockey turn (`turn/started` → frames → `turn/finished`), only marked with its origin. | Engine turn registry |

The consequence that keeps the views simple: **an external turn is just a turn.** The same
recorder (`engine::transcript`), the same events, the same renderer on desktop and web; the
only new UI is an origin badge and the owner state.

## Adapter contract

Extends `RuntimeAdapter` (`acp/session/adapter_runtime.rs`) the way `steer` did: optional
methods whose default returns `AdapterError::unsupported(<capability>)`, gated by new
`RuntimeCapabilities` fields so the engine and UI degrade on declared capability, never on
runtime names.

```rust
// RuntimeCapabilities (runtime_profile.rs)
observation: ObservationSupport,   // None | Tail | Live
shared_owner: bool,                // turns run through a vendor owner other surfaces attach to
ownership_signal: bool,            // the provider reports "another process owns this"
fork: bool,                        // existing

// RuntimeAdapter
async fn observe(&self, conv: &ConversationRef, from: Cursor, sink: ObservationSink)
    -> Result<ObservationHandle, AdapterError>;      // unsupported("observation")
async fn owner(&self, conv: &ConversationRef) -> Result<Owner, AdapterError>;
async fn fork(&self, key: &SessionKey, conv: &ConversationRef)
    -> Result<ConversationRef, AdapterError>;        // unsupported("fork")
```

- `observe` pushes frames into an `ObservationSink` that tags them with turn boundaries
  (`TurnBegin { origin, provider_turn_id }`, frames, `TurnEnd`) and the cursor after each
  entry. The adapter decides *how* (daemon notifications, file tail); the engine never sees
  files or sockets. Dropping the handle stops observing.
- `owner` answers from whatever the provider exposes: the daemon's thread state, Claude's
  "running in the background" refusal (cached from the last attempt), or `Unknown` from
  recent observed activity.
- Existing `prompt` is unchanged. A `shared_owner` runtime simply connects to the vendor
  owner instead of spawning a private process (Codex: `codex app-server proxy` instead of
  `codex app-server`).

| Runtime | observation | shared_owner | ownership_signal | fork | Implementation |
| --- | --- | --- | --- | --- | --- |
| `codex-cli` | Live | yes (daemon) | yes | yes | `thread/resume` subscription on the shared daemon; cursor = last item id |
| `claude-native` | Tail | no | yes | yes | Incremental JSONL tail, tolerant of unknown entry types; optional user-installed hook as change trigger; `--fork-session` |
| `pi-cli` | Tail | no | no | yes | Incremental JSONL tail; `--fork` |
| `claude-code` (ACP), `antigravity-cli`, custom ACP | None | no | no | per profile | No external surface to follow; `unsupported` |

## Engine (runtime-agnostic)

### Sequencer: one publish path with positions

Pattern: single writer per turn, "list-then-watch" for readers (as in Kubernetes informers
or event-sourced read models).

- `engine::stream::publish(origin, frame)` replaces the ~8 direct
  `emit("acp/delta" | "acp/stream")` sites (`headless.rs`, `native.rs`, `perm_bridge.rs`,
  the ACP worker) and is also the `ObservationSink`'s output. Under the turn's lock it applies
  the frame to the turn's `Transcript`, increments `rev`, and emits the frame with
  `{ turnId, rev, origin }`. Frames outside any turn (prewarm, probes) pass through unchanged.
- `get_live_turn(sessionId) -> { turnId, rev, origin, message } | null` (Tauri command,
  `GET /api/sessions/:id/live`) reads the in-memory transcript under the same lock, so the
  snapshot and `rev` are consistent by construction.
- Client join protocol, identical on desktop and web: subscribe, buffer, fetch the snapshot,
  apply frames with `rev > snapshot.rev`; a jump in `rev` means loss → refetch the snapshot.
  The web's `resync` becomes this refetch instead of a full reload.
- Side benefit: the current tap re-parses every frame's JSON (`listen_any` →
  `serde_json::from_str`); publishing hands the engine the structured value directly.

| Alternative | Why not |
| --- | --- |
| Poll the checkpoint more often | Lossy between polls; load grows with viewers. |
| Per-turn replay buffer of frames | Memory grows with the turn; long tool output replays megabytes to each joiner. |
| Stamp frames in the existing tap | The raw frame has already reached Tauri and the gateway; the stamp cannot follow it. |

### Observer: refcounted, read-only

- `watch_session(id)` / `unwatch_session(id)` (desktop on open/close; web tied to the SSE
  `focus`, released on disconnect). While the count is > 0 and the runtime declares
  `observation != None`, the engine holds one `ObservationHandle` from its last persisted
  cursor. Zero cost for sessions nobody is viewing.
- Observed `TurnBegin { origin: External }` opens an engine turn with that origin; frames go
  through `publish`; `TurnEnd` finishes it and persists with deterministic ids derived from the
  provider entries (the importer's `import:{runtime}:{id}` scheme), so a repeated or
  overlapping observation is idempotent. The cursor is persisted with the turn.
- **Own-write invariant.** Jockey's own turns also appear in the provider's log. The observer
  never records a turn the engine started: while `Owner = Jockey`, observed frames are
  dropped (Tail) or matched to the running Jockey turn by provider turn id (Live), and when a
  Jockey turn finishes the cursor is advanced past its entries.
- Observation never reserves the session, never spawns a provider process for `Tail`, and
  for `Live` only subscribes on a daemon that is already running.

### Write policy: viewing is free, sending is a lease

Pattern: single writer per conversation, readers never lock (CQRS). The only lock remains
`turns::reserve`; `submit` adds one decision based on `Owner`:

| Owner at submit | Action |
| --- | --- |
| `None` | Run. If external turns were observed since the provider slot's last turn, `discard_slot` first — a warm process would answer from a conversation that no longer matches the provider's record. |
| `Jockey` | Queue (existing behaviour). |
| `External { kind: Daemon }` | Queue behind the owner's turn; the daemon serialises turns on a thread, and the observer's `TurnEnd` drains the queue. |
| `External { kind: Background \| Unknown }` | Typed error `ConversationBusy { owner }`; views offer **wait** (queue until the observer sees the turn end, or a quiet period for `Unknown`), **fork** (`fork` → new conversation for this app session), or for `Background` an "attach in terminal" hint. |

No OS-level locking of provider files: the CLIs do not honour one, and it would only block
Jockey, which this policy already covers.

## Phases (full model)

The as-built mechanism covers the read side for every runtime. The phases below are what the
full model would add on top.

1. **Sequencer + snapshot** (requirement 1): `engine::stream::publish`, `rev`, `origin`,
   `get_live_turn` + `/api/sessions/:id/live`, the join protocol on desktop and web, `resync`
   → snapshot. Tests: transcript/`rev` consistency under interleaved frames, mid-turn join on
   both views, gap detection. No adapter changes beyond routing emits through `publish`.
2. **Contract + Codex Live** (requirement 2 for Codex): capability fields and trait methods;
   spike first — two daemon clients see each other's turns, `thread/resume` on a loaded thread
   subscribes rather than reloads — then `shared_owner` launch through
   `codex app-server proxy`, with the private process as fallback when no daemon runs.
3. **Observer + Tail** for `claude-native` and `pi-cli`: refcounted watcher (`notify`),
   incremental tail, cursor persistence, own-write invariant, origin badge; optional Claude
   hook trigger.
4. **Write policy**: `Owner` in summaries, `ConversationBusy`, stale-slot discard, fork flow,
   Claude background-session mapping.

Each phase is useful alone; phases 3–4 reuse phase 1's publish path unchanged.
