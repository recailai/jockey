# Zed's ACP Integration

> **Repo**: [zed-industries/zed](https://github.com/zed-industries/zed) (79.6k ★, Rust + GPUI)
> **Canonical file**: `crates/agent_servers/src/acp.rs` — 3,467 lines
> **SDK**: `agent-client-protocol` (same crate Jockey uses)
> **Snapshot**: file-size / line numbers verified April 2026

Zed is the project that originated ACP, so its Rust SDK integration is the reference implementation. It's also in the same general shape as Jockey — a foreground UI thread (GPUI) + background stdio to a CLI agent — so most patterns translate cleanly.

What follows is a structured read-through of Zed's ACP layer, grouped by subsystem. Each section has a code quote with a `file:line` pointer and a one-line "Jockey analogue/gap" note.

---

## 1. Connection lifecycle

Zed spawns one subprocess per ACP connection. There is **no pool** — each `AgentServer::connect()` call produces a fresh process and a fresh `ClientSideConnection`. When the process dies, every session bound to it receives a `LoadError::Exited` broadcast and is cleaned up.

**Subprocess spawning** (`acp.rs:540-548`):

```rust
let mut child = Child::spawn(child, Stdio::piped(), Stdio::piped(), Stdio::piped())?;
let stdout = child.stdout.take().context("Failed to take stdout")?;
let stdin  = child.stdin.take().context("Failed to take stdin")?;
let stderr = child.stderr.take().context("Failed to take stderr")?;
```

**Backpressured outgoing sink** (`acp.rs:588-603`) — the non-obvious piece:

```rust
let tapped_outgoing = futures::sink::unfold(
    (Box::pin(stdin), log_tap.clone()),
    async move |(mut writer, log_tap), line: String| {
        use futures::AsyncWriteExt;
        log_tap.emit_outgoing(&line);
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        Ok::<_, std::io::Error>((writer, log_tap))
    },
);
let transport = Lines::new(tapped_outgoing, tapped_incoming);
```

Each write `.await`s on `writer.write_all`, so if the child's stdin pipe is full the SDK's send loop yields instead of buffering unboundedly. `inspect()` on the incoming `BufReader::lines()` stream does the mirror-image tap for inbound traffic.

**Death detection** (`acp.rs:650-660`):

```rust
let wait_task = cx.spawn({
    let sessions = sessions.clone();
    let status_fut = child.status();
    async move |cx| {
        let status = status_fut.await?;
        emit_load_error_to_all_sessions(&sessions, LoadError::Exited { status }, cx);
        anyhow::Ok(())
    }
});
```

A dedicated background task awaits `child.status()` and broadcasts `LoadError::Exited` to every session in the map. Stderr is read on a separate task and forwarded to `log::warn!` + `log_tap.emit_stderr()` so it never blocks the protocol loop.

**Jockey gap**: `worker/pool.rs` has `health_rx: tokio::sync::watch::Receiver<bool>` for per-connection liveness but there's no broadcast-to-all-sessions pattern — each `handle_execute` instance polls its own `health_rx`. See [best_practices.md § transport-and-lifecycle](./best_practices.md) row "wait-task broadcast".

---

## 2. Handler dispatch + `into_foreground_future`

The SDK's handler trait methods are `Send`, but GPUI entities (`Entity<AcpThread>`, `Entity<Project>`) are `!Send` and can only be touched on the foreground app task. Zed bridges this with **an mpsc dispatch queue**: every handler closure pushes a `ForegroundWork` enum variant onto a channel; a foreground task pops them and runs the actual `!Send` work.

The subtle part is awaiting a nested ACP request from inside a handler. Zed's helper (`acp.rs:58-73`):

```rust
fn into_foreground_future<T: JsonRpcResponse>(
    sent: SentRequest<T>,
) -> impl Future<Output = Result<T, acp::Error>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let spawn_result = sent.on_receiving_result(async move |result| {
        tx.send(result).ok();
        Ok(())
    });
    async move {
        spawn_result?;
        rx.await.map_err(|_| {
            acp::Error::internal_error()
                .data("response channel cancelled — connection may have dropped")
        })?
    }
}
```

The doc comment on this function (which is worth reading in full in `acp.rs:41-57`) explains: the SDK gives you two ways to consume a `SentRequest` — `block_task()` (a normal `.await` in a spawned task) or `on_receiving_result()` (a callback invoked with the guarantee that no other inbound messages are processed while it runs). **Inside a handler callback, `block_task` deadlocks** because the SDK's dispatch loop is already holding the lock you'd need to process the response. `on_receiving_result` with a oneshot bridge is the recommended pattern — the callback itself is trivial (one channel send), so the ordering constraint it imposes is negligible.

**Jockey analogue/gap**: Jockey's `worker/` module runs on a dedicated single-thread Tokio runtime (`LocalSet`) precisely because `acp::ClientSideConnection` is `!Send`. That's structurally different from Zed's dispatch-queue approach but solves the same problem. The `on_receiving_result` vs. `block_task` distinction is still relevant inside Jockey's permission handler (`worker/permission.rs`) — worth auditing that we're not waiting on a nested ACP request via `.await` inside a handler callback.

---

## 3. Session HashMap + ref-counting

Sessions live in a `HashMap<SessionId, AcpSession>` wrapped in `Rc<RefCell<...>>` (single-threaded, so no `Arc`/`Mutex`). The struct is lean:

```rust
pub struct AcpSession {
    thread: WeakEntity<AcpThread>,                         // weak to prevent cycles
    suppress_abort_err: bool,
    models: Option<Rc<RefCell<acp::SessionModelState>>>,
    session_modes: Option<Rc<RefCell<acp::SessionModeState>>>,
    config_options: Option<ConfigOptions>,
    ref_count: usize,
}
```
(`acp.rs:278-285`)

**The race**: `load_session` takes time; during that time the user may click "close". Naively, close races with the in-flight load and can leave a dangling session. Zed resolves this with `ref_count`:

- `load_session` pre-registers the session with `ref_count: 1` before the RPC (so inbound notifications during history replay can find it).
- `close_session` decrements. If the load is still pending, the close RPC is **deferred** until the ref count hits zero.
- Concurrent loads of the same session id increment the ref count and share one entry.

The full lifecycle handling is in `acp.rs:830-950` (pending-sessions map) and `acp.rs:1368` (`close_session`).

**Jockey analogue/gap**: Jockey's `CONN_MAP` in `worker/pool.rs` is a plain `DashMap` keyed by `pool_key(app_session_id, runtime_key, role_name)` — one entry per active connection, no ref count. Because Jockey's prompt flow serializes through `PROMPT_LOCKS` per key, the load/close race is narrower than Zed's, but it exists when frontend code rapid-fires e.g. `reset_acp_session` followed by `assistant_chat`. Worth a thought experiment before we add any new "session state during cold-start" field.

---

## 4. Error classification: `FlattenAcpResult` + `map_acp_error`

Zed uses two complementary patterns to make ACP errors first-class in Rust's `Result<T, E>` plumbing.

**`FlattenAcpResult`** (`acp.rs:86-108`) — flattens nested `Result` shapes that come out of `entity.update(cx, |_, cx| fallible_op(cx))`:

```rust
trait FlattenAcpResult<T> {
    fn flatten_acp(self) -> Result<T, acp::Error>;
}

impl<T> FlattenAcpResult<T> for Result<Result<T, anyhow::Error>, anyhow::Error> {
    fn flatten_acp(self) -> Result<T, acp::Error> {
        match self {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err.into()),
            Err(err) => Err(err.into()),
        }
    }
}

impl<T> FlattenAcpResult<T> for Result<Result<T, acp::Error>, anyhow::Error> {
    fn flatten_acp(self) -> Result<T, acp::Error> {
        match self {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err),
            Err(err) => Err(err.into()),
        }
    }
}
```

The key insight is in the first impl: `anyhow::Error` values get converted via `acp::Error::from`, which internally downcasts an `acp::Error` back out of `anyhow` when present. **So typed errors like `AuthRequired` survive the anyhow round-trip.**

**`map_acp_error`** (`acp.rs:1626-1638`) — lifts the specific `AuthRequired` case into a typed domain error:

```rust
fn map_acp_error(err: acp::Error) -> anyhow::Error {
    if err.code == acp::ErrorCode::AuthRequired {
        let mut error = AuthRequired::new();
        if err.message != acp::ErrorCode::AuthRequired.to_string() {
            error = error.with_description(err.message);
        }
        anyhow!(error)
    } else {
        anyhow!(err)
    }
}
```

Every ACP RPC that could return auth-required goes through this. The UI then pattern-matches `err.downcast_ref::<AuthRequired>()` to open a login panel. Non-auth errors keep their `acp::Error` identity.

**Broadcast on fatal**: the `LoadError` enum (in `acp_thread/src/acp_thread.rs:1171-1200`) covers the non-recoverable cases — version mismatch, install failure, process exited — and is broadcast to all sessions via `emit_load_error_to_all_sessions` (`acp.rs:1019`).

**Jockey analogue/gap**: Jockey currently passes `Result<T, String>` everywhere. There's no `acp::Error` preserved, no `AuthRequired` typed error, no `LoadError` broadcast. This is the single highest-value pattern to port — AionUi built the TypeScript equivalent from scratch (see [`aionui.md § 6`](./aionui.md#6-error-handling)), and it's what an error-layer RFD for Jockey should be modeled on.

---

## 5. Logging tap: `AcpConnectionRegistry`

Zed has an in-app "ACP logs" panel that streams every JSON-RPC line for a selected connection. The key design constraint: **the tap must be ~free when nobody's subscribed**, otherwise every launched agent pays the observability cost.

Pattern (`acp.rs:565-580`):

```rust
let log_tap = cx.update(|cx| {
    AcpConnectionRegistry::default_global(cx).update(cx, |registry, cx| {
        registry.set_active_connection(agent_id.clone(), cx)
    })
});

let incoming_lines = futures::io::BufReader::new(stdout).lines();
let tapped_incoming = incoming_lines.inspect({
    let log_tap = log_tap.clone();
    move |result| match result {
        Ok(line) => log_tap.emit_incoming(line),
        Err(err) => log::warn!("ACP transport read error: {err}"),
    }
});
```

`log_tap.emit_*` methods are an atomic load + early return until the UI panel subscribes. When a user opens the panel, the registry swaps in a real subscriber and emits start flowing. Same wrapping on the `unfold` outgoing sink captures `emit_outgoing`.

**Jockey analogue/gap**: Jockey's `adapter::acp_log` (`acp/adapter.rs`) emits structured log lines to `eprintln!`/`tracing` unconditionally. Not expensive per line, but there's no user-facing inspect panel — adding one would require this opt-in-tap pattern so we don't pay the cost at steady state.

---

## 6. Protocol features used

**MCP servers injected on every session request** (`acp.rs:2783-2830`):

```rust
fn mcp_servers_for_project(project: &Entity<Project>, cx: &App) -> Vec<acp::McpServer> {
    let context_server_store = project.read(cx).context_server_store().read(cx);
    context_server_store
        .configured_server_ids()
        .iter()
        .filter_map(|id| {
            let configuration = context_server_store.configuration_for_server(id)?;
            match &*configuration {
                ContextServerConfiguration::Custom { command, remote, .. } if is_local || *remote
                    => Some(acp::McpServer::Stdio(
                        acp::McpServerStdio::new(id.0.to_string(), &command.path)
                            .args(command.args.clone())
                            .env(/* ... */)
                    )),
                ContextServerConfiguration::Http { url, headers, .. }
                    => Some(acp::McpServer::Http(
                        acp::McpServerHttp::new(id.0.to_string(), url.to_string())
                            .headers(/* ... */)
                    )),
                _ => None,
            }
        })
        .collect()
}
```

Called inside `new_session` (line 1117), `load_session` (line 1291), and `resume_session` (line 1337) — every session-starting RPC carries the full MCP server list. Zed supports both stdio and HTTP MCP servers; Jockey currently only supports stdio.

**Optimistic mode updates** (`acp.rs:1150-1270`): when a user switches session mode in the UI, Zed mutates the local `SessionModeState` **first**, then spawns an async task to send `SetSessionModeRequest`. If the request fails, it rolls back the local state:

```rust
modes_ref.current_mode_id = default_mode.clone();
cx.spawn({
    let default_mode = default_mode.clone();
    async move |_| {
        let result = into_foreground_future(
            conn.send_request(acp::SetSessionModeRequest::new(/* ... */))
        ).await.log_err();
        if result.is_none() {
            modes.borrow_mut().current_mode_id = initial_mode_id;
        }
    }
}).detach();
```

Zero-latency UI update, consistent fallback.

**Session fork**: not used in Zed. It uses `new_session` / `load_session` / `resume_session` only.

**Jockey analogue/gap**: Jockey's `session/cold_start.rs` already assembles MCP servers (via `session/mcp.rs::load_role_mcp_servers`). Optimistic mode updates are **not** in place — `set_acp_mode` in `session/session_cmds.rs` is strictly request-then-update. Low-value change, but worth noting for UX if mode-switching latency ever becomes a complaint.

---

## Summary — the four non-obvious Zed patterns

| Pattern | Quote location | Adopt effort for Jockey |
|---|---|---|
| `into_foreground_future` (handler + `on_receiving_result` + oneshot) | `acp.rs:58-73` | Audit only; Jockey already solves the same `!Send` problem via `LocalSet`. |
| Backpressured outgoing sink via `futures::sink::unfold` | `acp.rs:588-603` | Low — already applied in commit 44dc5dd (`apply backpressure on stream channel`). Cross-check. |
| `FlattenAcpResult` + `map_acp_error` downcast of `AuthRequired` | `acp.rs:86-108, 1626-1638` | **Medium — the highest-value port.** Jockey needs its own `AcpError` enum. |
| Opt-in log tap via `AcpConnectionRegistry` | `acp.rs:565-580` | Medium — depends on whether we build an in-app ACP logs panel. |

For the full checklist and cross-reference with AionUi, see [`best_practices.md`](./best_practices.md).
