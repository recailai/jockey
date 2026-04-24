# AionUi's ACP Integration

> **Repo**: [iOfficeAI/AionUi](https://github.com/iOfficeAI/AionUi) (22.5k ★, TypeScript + Electron)
> **Canonical folder**: `src/process/acp/` (multi-file, well-layered)
> **SDK**: `@agentclientprotocol/sdk`
> **Design docs** (inside the repo, worth reading): `docs/feature/acp-rewrite/02-reference-implementation.md` and `03-architecture-design.md`
> **Snapshot**: line numbers verified April 2026

AionUi is the closest project to Jockey in the ACP ecosystem — a desktop multi-agent orchestrator that spawns Claude/Gemini/Codex CLIs and bridges their ACP streams to a chat UI. Their recent `acp-rewrite` documented an explicit rebuild with Zed + OpenClaw acpx as references, so this file is effectively a distilled version of their design doc with the patterns that translate to Rust.

Structure below follows their module tree:

```
src/process/acp/
├── infra/           # transport + SDK wrapper + process
├── session/         # per-conversation state + execution
├── runtime/         # multi-session pool + idle GC
├── errors/          # typed hierarchy + recursive extractor + normalizer
└── metrics/         # latency / error / resume counters
```

---

## 1. "Single Owner" `ProcessAcpClient`

One class (`src/process/acp/infra/ProcessAcpClient.ts`, 503 lines) owns the subprocess, the SDK connection, the stderr buffer, the lifecycle state, and the pending-request set:

```typescript
export class ProcessAcpClient implements AcpClient {
  private child: ChildProcess | null = null;
  private connection: ClientSideConnection | null = null;
  private closing = false;

  // Stderr ring buffer (8KB, first-write-wins at failure)
  private stderrBuffer = '';

  // Lifecycle state (first-write-wins)
  private _lastExit: AgentExitInfo | null = null;
  private disconnectHandler: ((info: DisconnectInfo) => void) | null = null;
  private hasActivePrompt = false;

  // Pending request tracking — all SDK calls go through runConnectionRequest(),
  // which pushes into this set and rejects them on disconnect.
  private readonly pendingRequests = new Set<PendingRequest>();
```
(`ProcessAcpClient.ts:61-75`)

The invariant: **exactly one of these three states is live at any time** — `pre-start`, `started`, `exited`. `_lastExit` is the state machine's marker; its first write transitions `started → exited` and rejects every `pendingRequests` entry with `AgentDisconnectedError`.

**4-signal death detection** (`ProcessAcpClient.ts:355-366`):

```typescript
private attachLifecycleObservers(child: ChildProcess): void {
  child.once('exit', (code, signal) => {
    this.recordAgentExit('process_exit', code, signal);
  });
  child.once('close', (code, signal) => {
    this.recordAgentExit('process_close', code, signal);
  });
  child.stdout?.once('close', () => {
    this.recordAgentExit('pipe_close', child.exitCode ?? null, child.signalCode ?? null);
  });
  // connection_close is attached after ClientSideConnection is created (in start())
}
```

Plus a fourth signal wired in `start()` (`ProcessAcpClient.ts:128-130`):

```typescript
connection.signal.addEventListener(
  'abort',
  () => this.recordAgentExit('connection_close', child.exitCode ?? null, child.signalCode ?? null),
  { once: true }
);
```

**First-write-wins** (`ProcessAcpClient.ts:372-378`):

```typescript
private recordAgentExit(reason, exitCode, signal): void {
  if (this._lastExit) return;   // idempotent — only first signal wins
  // ... record exit info, reject pending requests ...
}
```

The `{ once: true }` on the listener is just belt-and-braces; the real idempotency guarantee is the early return on `this._lastExit`. Four separate event sources can race without producing double-cleanup.

**Jockey analogue/gap**: Jockey splits these responsibilities across `worker/pool.rs` (`LiveConnection` struct owns child process + SDK `Rc`), `worker/notify.rs` (death event sender), and `worker/handlers.rs` (watches `health_rx` during prompt). Only two signals are monitored: `child.status()` via `_io_task` and `health_rx` via `watch::Receiver<bool>`. **Missing**: `stdout.close` (stdin/stdout EOF before process exit) and `connection.signal.abort` (SDK-detected protocol abort). These tend to matter on gemini-cli / codex-cli which occasionally close the pipe without a clean exit.

---

## 2. `Promise.race(initialize, startupFailureWatcher)`

The classic bug: you spawn a process, call `initialize()`, and the process dies mid-handshake. `initialize()` either hangs forever or throws an opaque `connection closed` error that has no stderr context.

AionUi's fix (`ProcessAcpClient.ts:135-156`):

```typescript
async start(): Promise<InitializeResponse> {
  // ... spawn + attach lifecycle observers ...

  const startupFailure = this.createStartupFailureWatcher(child);
  try {
    const initResult = await Promise.race([
      this.runConnectionRequest(() =>
        this.conn.initialize({
          clientInfo: { name: 'AionUi', version: '2.0.0' },
          protocolVersion: PROTOCOL_VERSION,
          clientCapabilities: { fs: { readTextFile: true, writeTextFile: true } },
        })
      ),
      startupFailure.promise,
    ]);
    startupFailure.dispose();
    return initResult;
  } catch (err) {
    // normalize SDK's "connection closed" into AgentStartupError(stderr)
  }
}
```

The watcher (`ProcessAcpClient.ts:415-430`) races two signals:

```typescript
const onExit = (code, signal) => {
  if (disposed) return;
  rejectFn?.(new AgentStartupError(this.options.backend, code, signal ? String(signal) : null, this.stderrBuffer));
};
const onError = (err) => {
  if (disposed) return;
  rejectFn?.(new AgentSpawnError(this.options.backend, err));
};
child.on('exit', onExit);
```

`stderrBuffer` is captured **at rejection time**, not at cleanup time — so the user sees the real stderr that caused the startup failure, not the empty buffer after process exit tearing down IO.

**Jockey analogue/gap**: Jockey's cold-start path in `session/cold_start.rs` does not race `initialize` against process exit. The `PROMPT_LIVENESS_INTERVAL` heartbeat catches post-init death, but early-boot crashes (bad binary path, missing credential, first-token auth fail) currently surface as generic "connection disappeared after cold start" or `e.to_string()` noise. This is a small but high-value port: wrap the `initialize` future in a `tokio::select!` with a `child.wait()` arm and include stderr-tail in the error.

---

## 3. `AcpSession` vs. `ProcessAcpClient` (the layering)

AionUi's key design decision: **session-level state is not in the client**. The client is infrastructure; the session is business logic.

Rough sketch (synthesized from `src/process/acp/session/*.ts` and the arch doc):

```typescript
class AcpSession {
  // Owned sub-components
  private readonly configTracker = new ConfigTracker();
  private readonly permissionResolver = new PermissionResolver();
  private readonly messageTranslator = new MessageTranslator();
  private readonly promptExecutor = new PromptExecutor(this);
  private readonly lifecycle = new SessionLifecycle(this);
  private readonly inputPreprocessor = new InputPreprocessor();

  // State machine: idle → starting → active → prompting → suspended → resuming → error
  status: SessionStatus = 'idle';

  // Prompt serialization
  private readonly queue = new PromptQueue({ maxSize: 5 });

  // Does NOT own: the process, the SDK connection, the transport.
  // Those live inside this.lifecycle.client: ProcessAcpClient
}
```

**1:1:1 invariant**: one user conversation ↔ one `AcpSession` ↔ one agent process. Retries create new clients; failed clients are never reused. Session can outlive a client via `suspend()` + `resume()` (process exits, `sessionId` survives, new process is spawned and `loadSession` restores).

**Jockey analogue/gap**: Jockey's "session" is spread across:
- SQLite rows (`app_sessions`, `app_session_role`) — persistent state
- `CONN_MAP` entries in `worker/pool.rs` — in-memory per-connection state
- `runtime_state.rs` — discovered modes / config options cached per runtime
- Nothing binds these together as "the session object"

This is the biggest architectural delta. Not necessarily wrong — Jockey's role/session/workflow model is richer than AionUi's 1:1 conversation — but it does mean "where does the session's current mode live?" has three possible answers, depending on context. Worth a design note the next time we touch `runtime_state.rs`.

---

## 4. `PromptQueue` + `drainLoop`

Every ACP agent serializes prompts: you can't send a second `prompt` before the first completes. AionUi enforces this with a per-session FIFO (`AcpSession`):

```typescript
// Pseudo-code from arch doc
sendMessage(content) {
  this.queue.push(content);     // never executes directly
  if (!this.draining) this.drainLoop();
}

private async drainLoop() {
  this.draining = true;
  while (!this.queue.isEmpty) {
    const item = this.queue.pop();
    await this.promptExecutor.execute(item);   // serial
  }
  this.draining = false;
}
```

**Invariant INV-S-01** (from their design doc): only one prompt executes per session at any time, across the session's entire lifetime.

**PromptExecutor.execute** (sketch):

```typescript
async execute(content) {
  this.host.setStatus('prompting');
  try {
    await this.lifecycle.reassertConfig();    // apply pending desired mode/config first
    this.timer.start();
    await lifecycle.client.prompt(sessionId, content);   // streams via sessionUpdate handlers
    this.timer.stop();
  } catch (err) {
    this.handlePromptError(err, content);
  }
  this.host.messageTranslator.onTurnEnd();
  this.host.setStatus('active');
}
```

**Permission pause/resume** (integration with `PermissionResolver`):

```typescript
// When permission request arrives from agent:
this.promptExecutor.pauseTimer();
const outcome = await permissionResolver.evaluate(request);   // may block on UI
this.promptExecutor.resumeTimer();
return outcome;
```

Timer pause ensures **INV-S-04**: the prompt timeout only counts time the agent is actually working, not time the user spends staring at a "allow / deny" dialog.

**Jockey analogue/gap**: Jockey serializes prompts via `PROMPT_LOCKS` — a per-key `tokio::sync::Mutex` in `worker/pool.rs`. Same effect as the queue + drainLoop, but **the frontend sees queuing as "stuck"** — it can't tell the difference between "your prompt is next in line" and "the agent is thinking". AionUi's explicit queue lets them show queue position in the UI. Whether Jockey cares depends on product UX; for now, `PROMPT_LOCKS` + `cancel` is functionally equivalent.

---

## 5. `PermissionResolver` cascade

Three-level decision (`src/process/acp/session/PermissionResolver.ts:65-109`, sketch):

```typescript
async evaluate(request) {
  // Level 1 — YOLO: auto-approve everything
  if (this.yoloMode) {
    const allowOption = request.options.find(o => o.kind.startsWith('allow_'));
    return { outcome: { outcome: 'selected', optionId: allowOption?.optionId } };
  }

  // Level 2 — LRU cache: remember "always" decisions
  const cacheKey = buildCacheKey(request);     // kind + title + path/command from rawInput
  const cached = this.cache.get(cacheKey);
  if (cached) return { outcome: { outcome: 'selected', optionId: cached } };

  // Level 3 — UI callback with pending Promise
  const callId = toolCall.toolCallId;
  return new Promise((resolve, reject) => {
    this.pending.set(callId, { callId, resolve, reject, createdAt: Date.now(), cacheKey });
    uiCallback({ callId, title, description, kind, options, locations, rawInput });
  });
}

// Called from UI when user picks an option:
resolve(callId, optionId, rememberAlways) {
  const pending = this.pending.get(callId);
  if (!pending) return;
  this.pending.delete(callId);
  if (rememberAlways && optionId.startsWith('allow_')) {
    this.cache.set(pending.cacheKey, optionId);
  }
  pending.resolve({ outcome: { outcome: 'selected', optionId } });
}
```

Key points:
- **Only "allow_always" is cached** — denials are never cached, preventing a stale decision from locking the user out.
- **Pending map keyed by `callId`** — lets the UI route the user's decision back via a single string lookup, without holding references to Promise resolvers anywhere dangerous.
- **`cancelAll()` rejects all pending** — invoked when the session crashes mid-permission-wait, so the UI can clean up dialogs.

**Jockey analogue/gap**: Jockey's `worker/permission.rs` and `acp/client.rs` implement level-3 (UI callback with pending map) via a global `DashMap<String, oneshot::Sender<...>>` keyed by `request_id`. No level-1 (YOLO) and no level-2 (LRU allow-always cache). Adding either is localized to `worker/permission.rs` — low-risk port.

---

## 6. Error handling

The layer Jockey most conspicuously lacks. Three pieces work together.

### 6a. Typed hierarchy (`src/process/acp/errors/AcpError.ts`)

```typescript
export class AcpError extends Error {
  constructor(
    public readonly code: AcpErrorCode,
    message: string,
    options?: { cause?: unknown; retryable?: boolean }
  ) { super(message, { cause: options?.cause }); }
}

export class AgentSpawnError extends AcpError { /* CONNECTION_FAILED, retryable */ }
export class AgentStartupError extends AcpError { /* PROCESS_CRASHED, retryable */ }
export class AgentDisconnectedError extends AcpError { /* PROCESS_CRASHED, retryable */ }
```

`AcpErrorCode` enum is a union of:
- **Standard JSON-RPC**: `ACP_PARSE_ERROR` (-32700), `INVALID_ACP_REQUEST` (-32600), `ACP_METHOD_NOT_FOUND` (-32601), `ACP_INVALID_PARAMS` (-32602), `AGENT_INTERNAL_ERROR` (-32603)
- **ACP-specific**: `AUTH_REQUIRED` (-32000), `ACP_SESSION_NOT_FOUND` (-32001), `AGENT_SESSION_NOT_FOUND` (-32002), `ACP_ELICITATION_REQUIRED` (-32042), `ACP_REQ_CANCELLED` (-32800)
- **Application-layer**: `CONNECTION_FAILED`, `AUTH_FAILED`, `SESSION_EXPIRED`, `PROMPT_TIMEOUT`, `PROCESS_CRASHED`, `INVALID_STATE`, `INTERNAL_ERROR`, `AGENT_ERROR`

Every error eventually gets one of these codes + a `retryable: boolean`.

### 6b. Recursive payload extractor (`errorExtract.ts:15-37`)

```typescript
const MAX_DEPTH = 5;

export function extractAcpError(error: unknown, depth = 0): AcpErrorPayload | null {
  if (depth > MAX_DEPTH || error == null || typeof error !== 'object') return null;

  const obj = error as Record<string, unknown>;

  if (typeof obj.code === 'number' && typeof obj.message === 'string') {
    return {
      code: obj.code,
      message: obj.message,
      ...(obj.data !== undefined ? { data: obj.data } : {}),
    };
  }

  for (const key of ['error', 'cause', 'acp'] as const) {
    if (obj[key] != null) {
      const found = extractAcpError(obj[key], depth + 1);
      if (found) return found;
    }
  }

  return null;
}
```

Walks `error` → `cause` → `acp` up to 5 levels deep looking for the canonical `{ code, message, data? }` triple. Handles the three common nesting patterns seen across Node / SDK / MCP errors.

### 6c. Normalizer (`errorNormalize.ts`)

```typescript
export function normalizeError(error: unknown): AcpError {
  if (error instanceof AcpError) return error;

  // errno-based: ECONNREFUSED, ECONNRESET, EPIPE, ETIMEDOUT → retryable CONNECTION_FAILED
  if (error instanceof Error) {
    const errno = (error as NodeJS.ErrnoException).code;
    if (errno && RETRYABLE_ERRNO.has(errno)) {
      return new AcpError('CONNECTION_FAILED', error.message, { retryable: true });
    }
  }

  // SDK's RequestError → map via ACP_CODE_MAP
  if (error instanceof RequestError) {
    const mapped = ACP_CODE_MAP[error.code];
    // Heuristic: some agents return -32603 but the message is auth-related
    if (mapped && mapped.code !== 'AUTH_REQUIRED' && isAuthRelatedMessage(error.message)) {
      return new AcpError('AUTH_REQUIRED', error.message, { retryable: true });
    }
    return new AcpError(mapped.code, error.message, { retryable: mapped.retryable });
  }

  // Recursive fallback
  const acpPayload = extractAcpError(error);
  if (acpPayload) {
    const mapped = ACP_CODE_MAP[acpPayload.code];
    return new AcpError(mapped.code, acpPayload.message, { retryable: mapped.retryable });
  }

  return new AcpError('INTERNAL_ERROR', formatUnknownError(error));
}
```

Three layers of normalization: SDK-typed → errno-typed → recursively-extracted → generic fallback. Every function in the ACP layer funnels its errors through this so callers get a canonical `AcpError` with structured fields.

**Jockey analogue/gap**: Jockey has **none** of this. Every fallible ACP operation returns `Result<T, String>` and every call site does `.map_err(|e| e.to_string())`. String-based error handling means:
- No `retryable` flag — retry logic is ad-hoc per call site
- No auth-required detection at the ACP layer — UI code does `if err_msg.contains("auth")`
- No way to preserve `acp::Error`'s structured `code` + `data` fields

Porting this should be the top-priority post-MVP cleanup. See [`best_practices.md`](./best_practices.md) row "typed error enum".

---

## 7. `AcpRuntime` + `IdleReclaimer`

Multi-session management. `src/process/acp/runtime/AcpRuntime.ts` holds:

```typescript
type SessionEntry = { session: AcpSession; lastActiveAt: number };

class AcpRuntime {
  private readonly sessions = new Map<string, SessionEntry>();
  private readonly idleReclaimer: IdleReclaimer;

  constructor(clientFactory, options?) {
    this.idleReclaimer = new IdleReclaimer(
      this.sessions,
      options?.idleTimeoutMs ?? 300_000,   // 5 min
      options?.checkIntervalMs ?? 30_000   // 30 sec
    );
    this.idleReclaimer.start();
  }
}
```

Every `sendMessage()` updates `entry.lastActiveAt = Date.now()`. The reclaimer scans every 30 seconds:

```typescript
private scan() {
  const now = Date.now();
  for (const [_, entry] of this.sessions) {
    if (entry.session.status === 'active' && now - entry.lastActiveAt > this.idleTimeoutMs) {
      entry.session.suspend().catch(/* ... */);
    }
  }
}
```

**Suspend, don't kill**: `session.suspend()` gracefully closes the SDK connection and child process, but **preserves the sessionId**. Next `sendMessage()` calls `session.resume()` which spawns a fresh process and issues `loadSession(savedSessionId)` to restore history.

**Jockey analogue/gap**: Jockey's `CONN_MAP` in `worker/pool.rs` evicts entries only on cwd change or error. There's no time-based reclamation — a connection that's been idle for a week still holds a live subprocess. Adding an idle reclaimer is a decent mid-priority port; the harder part is the "suspend & resume via loadSession" workflow, which Jockey doesn't currently model.

---

## 8. `ConfigTracker`: desired vs. current

Every mode or config-option change has **two states**:

```typescript
class ConfigTracker {
  // Current — confirmed by agent, set from protocol responses only
  private currentModeId: string | null = null;
  private currentConfigOptions: ConfigOption[] = [];

  // Desired — user intent, not yet synced
  private desiredModeId: string | null = null;
  private desiredConfigOptions = new Map<string, string | boolean>();

  setDesiredMode(id: string) { this.desiredModeId = id; }
  setCurrentMode(id: string) {
    this.currentModeId = id;
    if (this.desiredModeId === id) this.desiredModeId = null;   // cleared after confirm
  }

  getPendingChanges(): PendingChanges { /* diff desired vs current */ }
  clearPending() { this.desiredModeId = null; this.desiredConfigOptions.clear(); }
}
```

Reconciliation in `SessionLifecycle.reassertConfig()` runs **before every prompt**:
1. Compute `tracker.getPendingChanges()`
2. For each pending: call `client.setMode()` / `client.setConfigOption()`
3. On success: `tracker.setCurrentMode(...)` clears the desired half

The model has two nice properties:
- UI can read `desired` immediately after a click (responsive)
- On session restart / resume, pending changes get re-sent automatically via `reassertConfig`
- If a config set fails, `desired` persists so the user sees what they asked for and an error banner, instead of silent rollback

**Jockey analogue/gap**: Jockey's `set_acp_mode` / `set_acp_config_option` in `session/session_cmds.rs` is synchronous request-then-persist. No split. The SQLite side (`app_session_role.mode_override`) is the "desired" in spirit, and the runtime-state cache is the "current", but nothing reconciles them. For now this is fine because Jockey doesn't have the same "suspend/resume across crashes" story; revisit when we add idle reclaim.

---

## 9. `AuthNegotiator`

Auth is env-var based in ACP spec: the `authenticate(methodId)` RPC tells the agent which method to use, but the credentials themselves live in the child process's environment variables (set at `spawn` time).

```typescript
class AuthNegotiator {
  private credentials: Record<string, string> | null = null;

  mergeCredentials(creds: Record<string, string>) { /* ... */ }

  async authenticate(protocol, authMethods?) {
    const selected = this.selectAuthMethod(authMethods ?? []);
    if (!selected) return;              // no matching method — skip
    await protocol.authenticate(selected.id);
  }

  private selectAuthMethod(methods): AuthMethod | null {
    for (const method of methods) {
      if (method.type !== 'env_var') continue;
      const allPresent = method.vars.every(v => this.credentials?.[v.name]);
      if (allPresent) return method;    // first matching wins
    }
    return null;
  }
}
```

Auth flow:
1. `client.start()` returns `InitializeResponse` with optional `authMethods`.
2. `AuthNegotiator.authenticate()` picks the first method whose required env vars are all present; if none match, silently skip (the agent may auth internally via other means).
3. If `initialize` throws `AUTH_REQUIRED` anyway → stop startup, emit `onSignal({ type: 'auth_required' })`.
4. UI shows login UI → user provides creds → `session.retryAuth(creds)` merges into negotiator and calls `resume()` which spawns a fresh process with the new env.

**Session-scoped credentials**: creds live on the `AuthNegotiator` instance, not globally. Two conversations using the same backend with different credentials don't clash.

**Jockey analogue/gap**: Jockey's auth story is fully external — users log in via `claude login` / `gemini auth` outside the app, and Jockey spawns with the ambient env. No in-app auth negotiation. For Jockey's target users (devs with pre-configured CLIs) this is fine; for broader distribution it's a gap.

---

## 10. `AcpMetrics`

Minimal, interface-only (recording is no-op by default — Phase 1):

```typescript
export type AcpMetrics = {
  recordSpawnLatency(backend: string, ms: number): void;
  recordInitLatency(backend: string, ms: number): void;
  recordFirstTokenLatency(backend: string, ms: number): void;
  recordError(backend: string, code: AcpErrorCode): void;
  recordResumeResult(backend: string, success: boolean): void;
  snapshot(): MetricsSnapshot;
};
```

Recording points:
- `spawn` + `init`: `SessionLifecycle.doStart()` around `client.start()`
- `first_token`: on first `sessionUpdate` carrying text content
- `error`: `PromptExecutor.handlePromptError` (after normalize)
- `resume_result`: `SessionLifecycle.doResume()` completion

**Why interface-only matters**: the design cost is paid once (deciding what to measure), implementation can be swapped in later (local aggregator, OTel exporter, in-app dashboard) without touching call sites.

**Jockey analogue/gap**: Jockey has `adapter::acp_log` emitting structured logs (`stage.ok`, `pool.invalidate`, etc.) but no aggregated metrics interface. Adding a thin `AcpMetrics` trait with a no-op default would let us collect latency percentiles without a big refactor.

---

## Summary — the three highest-value AionUi patterns for Jockey

| Pattern | Upstream location | Effort to port |
|---|---|---|
| Typed `AcpError` hierarchy + `extractAcpError` + `normalizeError` | `errors/*.ts` (~200 lines total) | **High value, medium effort.** Would touch every `.map_err(|e| e.to_string())` call site in Jockey. |
| 4-signal death detection + `Promise.race(initialize, startupFailureWatcher)` | `ProcessAcpClient.ts:355-430` | **Medium value, low-medium effort.** Localized to `worker/pool.rs` + cold-start. |
| `IdleReclaimer` with suspend-not-kill | `runtime/IdleReclaimer.ts` (~40 lines) | **Low-medium value, medium effort.** Needs "save sessionId + re-init via loadSession" workflow that Jockey doesn't have yet. |

See [`best_practices.md`](./best_practices.md) for the full checklist and side-by-side with Zed.
