# Session sync: late joiners and external CLI activity

Status: read-side sync implemented (see [As built](#as-built)); the adapter-contract
extensions below (Codex shared daemon, sequenced frames) remain design. Builds on
[engine-service-layer.md](engine-service-layer.md) and the adapter contract in
[runtime-adapters.md](runtime-adapters.md).

## As built

The requirement is read-only following with eventual consistency, plus a running state, for
every provider conversation a session is bound to, open or closed. The implementation picks the
cheapest mechanisms that are exact at the end of every turn.

| Concern | Options weighed | Chosen |
| --- | --- | --- |
| View joins a running Jockey turn | (a) sequence every frame (`rev`) through an engine publisher; (b) clients poll the checkpoint; (c) **notify, then pull**: a tiny `turn/progress { turnId, rev }`, late joiners fetch the in-memory snapshot | (c), unchanged. |
| Provider conversation driven elsewhere | (a) join the vendor owner (Codex daemon); (b) **OS file watcher (`notify`) on the transcript roots, debounced**, with a stat sweep as fallback and a startup sweep; (c) stat only the shown session | (b)+sweep for *all* bound sessions, so the sidebar and the phone companion stay current without anyone focusing a session. (a) changes how turns run. |
| Where reading resumes | (a) entry count over a re-parsed projection (the old `transcript_cursor`); (b) **byte offset + fingerprint per binding** | (b): only appended bytes are read; a 40 MB Claude transcript is never re-parsed per change. |
| Shape of an imported turn | (a) one message per transcript entry; (b) **one assistant message per provider turn, in the payload `TurnRecorder` writes** | (b): text, thought and tool calls (title, kind, status, raw input/output, bounded) render exactly like a turn Jockey ran. Built with the engine's own `Transcript` reducer. |
| In-progress external turn | (a) persist and update in place; (b) **hold until the end marker, serve a live preview** | (b): stored messages stay immutable and appends stay append-safe. The preview is served through the live-turn pull (`engine::watch::live_turn`, `turn/progress`) and settled with `turn/finished` carrying the stored message. |
| Running state | (a) recent-write window; (b) **the provider's own turn markers** | (b), with a time window only for a runtime that has no markers. |

### Bindings

A binding is a row of `app_session_role_runtime_configs`: `acp_session_id` (the provider
handle) plus `transcript_offset` / `transcript_fingerprint`. Closing a session no longer erases
the handle (only the warm process is released); only the explicit `reset_acp_session` drops it,
and it also detaches the imported id so the binding is not restored behind the user's back.
`reopen_app_session` rebinds from `app_sessions.external_session_id` when the handle is missing,
`list_provider_conversations` falls back to the same id, and schema v19 repairs sessions that an
older close left with an imported id but no handle.

### Reading a transcript (`importer::tail`, `importer::sync`)

- **Position.** `offset` is the end of the last fully handled line (zstd frame for dsh). The
  fingerprint is a hash of the file's first bytes (inside the handled region) and of the 128
  bytes before `offset`. A sync verifies it with two small reads; a shrunk file or a mismatch
  triggers a full reconcile from byte 0 instead of jumping to the end. The ids make that safe.
  A fingerprint cannot see an edit in the middle of a long file; transcripts are append-only,
  so that is accepted.
- **Torn writes.** A trailing partial line (or zstd frame) is not consumed; the next read
  starts at its beginning. A complete line that does not parse is stepped over.
- **Ids.** Provider-native, so they survive parser changes: `import:{runtime}:{provider
  id}:u:<uuid>` for a prompt, `:t:<first assistant entry uuid>` for a turn, `:e:<uuid>` for a
  task-notification event (Codex: `t:<turn_id>`, Pi: entry ids, dsh: event `seq`). The unique
  index `(session_id, client_id)` makes any re-read idempotent.
- **Turns.** Each reducer groups entries into prompts, events and turns:
  - Claude: one entry per content block; `stop_reason` is repeated on each block of a message,
    so a turn closes on the entry that carries the reply (a thinking block marked `end_turn`
    does not close it), on `system/turn_duration`, on `[Request interrupted by user]`, or on
    the next prompt. Sidechain entries are dropped (never part of the parent's reply). The
    request mirrors (`api-request*`), attachments and bookkeeping entries are skipped before
    JSON parsing.
  - Codex: `task_started` .. `task_complete` / `turn_aborted`; marker-less old rollouts are
    stored whole. Pi: assistant `stopReason` (`toolUse` continues). dsh: `turn/start` ..
    `turn/end`, one frame per record.
  - A tool without a result when its turn closes is shown as interrupted.
- **Open turn.** The reducer reports where the open turn starts; the next read begins there
  (bounded by the turn, not the file) and the turn is stored only when it closes. A turn
  without an end marker in a file quiet for 10 minutes is stored as finished.

### Migrating existing sessions

A binding without a byte position (everything imported before this change) is migrated on
its first sync. Mapping the old entry count to a byte offset would mean re-running the old
projections, so instead:
- a session holding nothing but imported rows is **upgraded**: its positional rows
  (`…:<index>`) are replaced, in one transaction, by turn-shaped rows read from the start of the
  transcript (tool calls included);
- a session Jockey also wrote to keeps its rows; transcript items no newer than the session's
  latest recorded activity (its newest message or `last_active_at`, plus 5 s) are treated as
  already covered (imported, or Jockey's own turns, which the transcript records too) and
  newer ones are imported. That time is an upper bound of what the old cursor covered, because
  old imports were stamped at import time and every Jockey save bumps `last_active_at`.

### Own writes

Jockey's turns also appear in the provider's transcript. `turns::begin` opens an *own
window* (`start`, `end` once the turn finishes) and the sync skips transcript items inside it
(with 0.5 s lead and 3 s trail for the CLI's flush), instead of moving the cursor to the end
after the turn. Syncing continues during a Jockey turn, at most every 5 s (each read re-reads
the turn in progress); entries written outside the window by someone else are imported and
discard the warm provider slot, so the next Jockey turn resumes from the provider's record. A
sync runs once after the settle time. `submit` still refuses input while `externalActive`.

### Running state

Per binding, the last marker the reducer saw: running while a turn is open (prompt, tool
result, `stop_reason: tool_use`, `task_started`, `turn/start`), idle at the end marker.
`externalActive` is true while any binding is running, but a marker only counts while the
file was written within the last 10 minutes (a crashed CLI leaves its last marker behind, and
this state refuses input), and never while a Jockey turn runs or just ended. A runtime
without markers falls back to "transcript grew in the last 8 s". Changes are announced with
`session/changed`.

### Pieces

- `engine::watch::start` (called at setup): the manager. Jobs: file events (debounced 300 ms),
  a 30 s stat sweep (also re-evaluates running state, since a trust window can lapse without a
  file event), a startup sweep over every binding, and `schedule_sync` (after an import or a
  Jockey turn). Roots: `~/.claude/projects` (or `$CLAUDE_CONFIG_DIR`), `~/.codex/sessions`,
  `~/.pi/agent/sessions`, `~/.dsh/sessions` (or `$DSH_HOME`). If the OS watcher cannot start,
  the sweep alone keeps everything current.
- `engine::watch::{watch, unwatch}`: refcounted, per shown session (desktop: the shown
  session; web: the SSE `focus`); one loop announces a Jockey turn's `turn/progress` and keeps a
  1 s sync for the open session.
- `turns::{live_rev, live_turn}` plus `watch::live_turn`, which falls back to the external
  turn's preview; served by `get_live_turn`, `watch_session` and `GET /api/sessions/:id/live`.
- Events: `message/added` for stored items, `turn/progress` for the preview,
  `turn/finished` (with the stored message) when an announced external turn ends,
  `session/changed` for summary and running-state changes.

Not covered: Antigravity and custom ACP runtimes have no transcript reader. Codex's shared
daemon and sequenced frames below remain design.

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
