# Session sync: late joiners and external CLI activity

Status: design, not implemented. Builds on [engine-service-layer.md](engine-service-layer.md).

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
| Viewing takes ownership | Already true that it does not: only `submit` takes the `turns::reserve` slot; config discovery uses a separate probe connection (`prewarm_role_config_cmd`). | Keep it that way (rule below). |

The engine already holds the one thing a late joiner needs: the running turn's transcript
(`turns::ActiveTurn.transcript`), updated under a lock by the frame tap. What is missing is a
**position** shared by that state and the frames.

## How the agent CLIs handle it (checked 2026-10-19)

None of them supports two independent processes writing one conversation. The pattern they
converge on is **one owner process per conversation, other surfaces attach to it**.

| CLI | Two processes resume one session | What the vendor offers instead |
| --- | --- | --- |
| Claude Code 2.1.x | Documented as unsafe: "If you resume the same session in two terminals without forking, messages from both interleave into one transcript." | **Background sessions** (`claude --bg`, `claude agents`, `claude attach <id>`), hosted by a per-user supervisor: resuming a *running* background session attaches to that process instead of loading the transcript. A `claude -p --resume <id>` that sets output/config flags (as Jockey does) is refused with exit 1 and the `claude attach` hint. `--fork-session` / `/branch` copy the conversation. Remote Control lets a phone follow a session. The transcript JSONL is documented as an internal format that "changes between versions"; hooks receive `transcript_path` and are the supported trigger. |
| Codex 0.163 | Each `codex app-server` process loads the thread on its own. | A **shared local app-server daemon** (`codex app-server daemon start`, `codex agents`, `codex queue --thread`, TUI `--remote`). Clients of the same daemon subscribe to one in-memory thread and all receive `turn/*` notifications; `thread/resume` rebuilds state for a reconnecting client. `codex app-server proxy` bridges stdio to the daemon's control socket. `codex fork` / `thread/fork` branch a thread. (Multi-client semantics come from third-party write-ups; verify before relying on them.) |
| Pi 1.1 | Plain JSONL session files, no owner process. | `--fork <id>` into a new session; no daemon. |

What this changes in Jockey's design:

- Jockey's engine already *is* this pattern internally (single owner per session, views
  subscribe). Section 1 below makes late joining lossless — the same contract Codex
  implements with `thread/resume`.
- For Codex the better answer than tailing files is to **join the vendor's owner**: run
  turns through the shared daemon (`codex app-server proxy`) instead of a private
  `codex app-server`, and a terminal TUI on the same daemon becomes another subscriber.
- For Claude and Pi there is no local multi-client API Jockey can use, so following an
  external session is read-only tailing (section 2) and writing must avoid a concurrent
  owner (section 3). Because the Claude transcript format is internal, the tailer must be
  tolerant, and hooks are the preferred change trigger where the user opts in.

## Design

### 1. Snapshot + sequenced stream (late joiners)

Pattern: the "list-then-watch" contract (as in Kubernetes informers, or event-sourced read
models): a client fetches state at version *N*, subscribes, and applies only events with
version > *N*.

| Option | Problem |
| --- | --- |
| Clients refetch the checkpoint more often | Still lossy between polls; load grows with viewers. |
| Replay buffer of recent frames per turn | Memory per turn grows with the turn; a long tool output replays megabytes to every joiner. |
| Stamp frames in the existing tap, keep adapters emitting raw events | The raw frame has already left (Tauri and the gateway see it before or alongside the tap), so the stamp cannot reach the clients that need it. |
| **Engine becomes the sequencer: adapters hand frames to the engine, which applies, stamps and publishes** | — |

Chosen: a single writer per turn.

- `engine::stream::publish(frame)` replaces the ~8 direct `emit("acp/delta" | "acp/stream")`
  sites (`acp/session/headless.rs`, `native.rs`, `perm_bridge.rs`, the ACP worker). Under the
  turn's lock it applies the frame to `Transcript`, increments `rev`, and emits the frame
  with `{ turnId, rev }` added. Frames for sessions with no engine turn (prewarm, probes) are
  emitted unchanged.
- `get_live_turn(sessionId) -> { turnId, rev, message } | null` (Tauri command and
  `GET /api/sessions/:id/live`) returns the in-memory transcript rendered as the same
  `AppMessage` shape `turn/finished` persists, read under the same lock — so `rev` and the
  snapshot are consistent by construction.
- Client protocol: subscribe first, buffer frames, fetch the snapshot, render it, apply
  buffered and later frames with `rev > snapshot.rev`, drop the rest. A jump in `rev`
  (`rev > last + 1`) means frames were lost — refetch the snapshot instead of guessing.
  The web's `resync` becomes the same refetch, not a full reload.
- `turn/finished` is unchanged and stays authoritative: it replaces whatever the view built.

Side benefit (performance): the tap today re-parses every frame's JSON string
(`listen_any` → `serde_json::from_str`). Publishing through the engine hands it the
structured value directly, one parse fewer per token on the hottest path.

Desktop latency is unaffected: the desktop keeps rendering frames as they arrive; it only
gains the ability to *join* a turn it did not see start (`engineEvents.ts` opens the live
projection from the snapshot instead of ignoring frames).

### 2. External activity: tail the provider transcript (read-only)

Pattern: change-data-capture by tailing the provider's own log. The provider transcript file
(`~/.claude/projects/…/<id>.jsonl`, Codex rollout, Pi session) is the source of truth for
anything that happened outside Jockey; Jockey already knows each imported session's file
(`import_session_catalog.file_path`) and parses it idempotently.

| Option | Problem |
| --- | --- |
| Periodic re-import of every session | Rescans files nobody is looking at; latency of the poll interval. |
| Attach to the running CLI process | No supported API for any of the CLIs; would take ownership. |
| Watch whole provider directories | Thousands of files; events for unrelated projects. |
| **Watch only the files of sessions someone is viewing; parse appended bytes only** | — |

Chosen:

- **Refcounted subscriptions.** A view opening a session calls `watch_session(id)` and
  releases it on close (web: tied to the SSE `focus`, released on disconnect). The engine
  watches a file only while its refcount is > 0 — one OS watch per viewed session, zero cost
  for the rest.
- **Incremental tail.** Keep `(file, byte_offset, last_entry_id)` per watched file; on a
  debounced (~300 ms) change event, read from the offset, parse complete lines only, and
  upsert through the existing importer path (deterministic `import:{runtime}:{id}` ids, so a
  repeated or overlapping read is harmless). Emit `message/added` per new entry and
  `session/changed` for the summary. A truncated/rotated file (size < offset) falls back to
  a full idempotent re-import.
- **Attribution.** While the engine runs a turn in that session, file growth is Jockey's own
  write and is ignored by the tail (the turn's record comes from the engine). Growth with no
  engine turn is external: the session gets `externalActivityAt` in its summary, and views
  show "active in terminal".
- Requires a file-watch dependency (`notify`, FSEvents on macOS); nothing in the tree
  watches files today.
- **Per runtime.** Codex: not needed once turns go through the shared daemon (the daemon's
  thread notifications are the live feed). Claude: tail, optionally triggered by a
  user-installed hook (`Stop` / `UserPromptSubmit` posting to the local gateway) instead of
  polling the file system; the parser skips entry types it does not know rather than failing,
  since the format is internal. Pi: tail.

### 3. Ownership: viewing is free, sending is a lease

Pattern: single writer per session, readers never lock (CQRS). Viewing = queries and
subscriptions only. Sending = the existing `turns::reserve` slot, which is the only lock.

External activity adds one rule at the write side, in `engine::service::submit`:

- **Stale warm process.** If `externalActivityAt` is newer than the provider slot's last
  turn, evict the slot before running (`discard_slot`): the next turn re-resumes from the
  provider's file and sees the terminal's messages. Without this, a warm `claude` or
  `codex app-server` process answers from a conversation that no longer matches the file.
- **Concurrent writer.** If external activity is *recent* (within ~10 s, i.e. the terminal is
  probably mid-turn), do not resume the same provider session. Reject with a typed error the
  views turn into a choice: wait, or **fork** — Claude `--resume <id> --fork-session`, Codex
  thread fork (`fork_provider_session_cmd` already exists for native Codex). Pi and agy: wait
  only.
- No OS-level locking of provider files: the CLIs do not honour one, and an advisory lock
  would only block Jockey, which this rule already covers.
- **Vendor ownership signals come first.** Claude refusing a resume of a running background
  session (exit 1, "running in the background") maps to the same typed error, with
  "attach in terminal" / fork as the choices. On the Codex daemon the daemon itself
  serialises turns on a thread, so the rule reduces to queueing behind its running turn.

## Phases

1. **Sequenced stream + snapshot** (requirement 1): `engine::stream::publish`, `rev`,
   `get_live_turn` + `/api/sessions/:id/live`, client join protocol on desktop and web, and
   `resync` → snapshot refetch. Tests: transcript/rev consistency under interleaved frames,
   join-mid-turn on both views, gap detection.
2. **Codex on the shared daemon** (requirement 2 for Codex, live and lossless): spike first —
   confirm that two clients of `codex app-server daemon` see each other's turns and that
   `thread/resume` on a loaded thread subscribes rather than reloads; then launch through
   `codex app-server proxy` with the private process as fallback.
3. **Watch + tail for Claude and Pi** (read-only): refcounted watcher, incremental tail
   through the importer, `externalActivityAt`, "active in terminal" badge; optional Claude
   hook trigger.
4. **Write-side rules** (correctness when both sides talk): evict stale slots, reject-or-fork
   on recent external activity, map Claude's "running in the background" refusal.

Each phase is useful alone; phase 4 depends on phase 3's attribution.
