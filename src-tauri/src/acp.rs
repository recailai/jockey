/// ACP transport layer using the official `agent-client-protocol` Rust crate.
///
/// `ClientSideConnection` futures are `!Send` (they use `Rc` internally), so all live
/// connections must run on a single-threaded runtime.  We spin up one dedicated OS thread
/// with a `current_thread` Tokio runtime + `LocalSet`; external callers communicate with it
/// via `tokio::sync` channels that cross thread boundaries safely.
use agent_client_protocol::{self as acp, Agent as _};
use dashmap::DashMap;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use which::which;

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    Mock,
    ClaudeCode,
    GeminiCli,
    CodexCli,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPromptResult {
    pub output: String,
    pub deltas: Vec<String>,
    pub meta: Value,
}

// ── ACP worker thread ─────────────────────────────────────────────────────────

/// Messages sent from async callers into the ACP worker thread.
enum WorkerMsg {
    Execute {
        runtime_key: &'static str,
        binary: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        role_name: String,
        prompt: String,
        context: Vec<(String, String)>,
        cwd: String,
        /// Receives streaming delta chunks while the prompt is in flight.
        delta_tx: mpsc::UnboundedSender<String>,
        /// Returns the final result.
        result_tx: oneshot::Sender<Result<(String, String), String>>,
    },
    Prewarm {
        runtime_key: &'static str,
        binary: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        role_name: String,
        cwd: String,
    },
}

/// One worker thread for all ACP connections.
static WORKER_TX: OnceLock<mpsc::UnboundedSender<WorkerMsg>> = OnceLock::new();

fn worker_tx() -> &'static mpsc::UnboundedSender<WorkerMsg> {
    WORKER_TX.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel::<WorkerMsg>();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("acp worker runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, run_worker(rx));
        });
        tx
    })
}

// ── worker loop ───────────────────────────────────────────────────────────────

fn pool_key(runtime_key: &str, role_name: &str) -> String {
    format!("{runtime_key}:{role_name}")
}

/// Per-slot mutex serializes requests to the same agent session.
/// Wrapped in Arc so it can be shared across spawn_local tasks.
struct SlotHandle {
    /// The live connection, guarded so only one prompt runs at a time per slot.
    conn: tokio::sync::Mutex<Option<LiveConnection>>,
}

/// Global slot map — lives on the worker thread, but Arc allows spawn_local tasks to hold refs.
static SLOT_MAP: OnceLock<Arc<DashMap<String, Arc<SlotHandle>>>> = OnceLock::new();

fn slot_map() -> &'static Arc<DashMap<String, Arc<SlotHandle>>> {
    SLOT_MAP.get_or_init(|| Arc::new(DashMap::new()))
}

fn get_slot_handle(runtime_key: &str, role_name: &str) -> Arc<SlotHandle> {
    let key = pool_key(runtime_key, role_name);
    slot_map()
        .entry(key)
        .or_insert_with(|| Arc::new(SlotHandle { conn: tokio::sync::Mutex::new(None) }))
        .clone()
}

struct LiveConnection {
    conn: acp::ClientSideConnection,
    session_id: acp::SessionId,
    cwd: String,
    /// Shared with the UnionAiClient inside conn — swap sender on each request.
    delta_slot: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>,
    _child: tokio::process::Child,
    _io_task: tokio::task::JoinHandle<()>,
}

async fn run_worker(mut rx: mpsc::UnboundedReceiver<WorkerMsg>) {
    // Drain messages and dispatch each as an independent spawn_local task.
    // Different slots run fully concurrently; same slot is serialized by SlotHandle::conn mutex.
    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMsg::Execute { runtime_key, binary, args, env, role_name, prompt, context, cwd, delta_tx, result_tx } => {
                let slot = get_slot_handle(runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    handle_execute(slot, runtime_key, binary, args, env, role_name, prompt, context, cwd, delta_tx, result_tx).await;
                });
            }
            WorkerMsg::Prewarm { runtime_key, binary, args, env, role_name, cwd } => {
                let slot = get_slot_handle(runtime_key, &role_name);
                tokio::task::spawn_local(async move {
                    handle_prewarm(slot, runtime_key, binary, args, env, role_name, cwd).await;
                });
            }
        }
    }
}

async fn handle_execute(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    prompt: String,
    context: Vec<(String, String)>,
    cwd: String,
    delta_tx: mpsc::UnboundedSender<String>,
    result_tx: oneshot::Sender<Result<(String, String), String>>,
) {
    let mut guard = slot.conn.lock().await;
    let resolved = resolve_cwd(&cwd);

    // Evict on cwd change
    if guard.as_ref().map(|c| c.cwd != resolved).unwrap_or(false) {
        acp_log("pool.evict", json!({ "runtime": runtime_key, "role": role_name, "reason": "cwd_change" }));
        *guard = None;
    }

    if guard.is_none() {
        match cold_start(&binary, &args, &env, &resolved).await {
            Ok(conn) => {
                acp_log("pool.cold_start", json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }));
                *guard = Some(conn);
            }
            Err(e) => {
                let _ = result_tx.send(Err(e));
                return;
            }
        }
    } else {
        acp_log("pool.reuse", json!({ "runtime": runtime_key, "role": role_name }));
    }

    let conn = guard.as_mut().unwrap();
    let session_id = conn.session_id.clone();

    // Install delta sender for this request
    if let Ok(mut slot) = conn.delta_slot.lock() { *slot = Some(delta_tx); }

    let mut blocks: Vec<acp::ContentBlock> = vec![
        acp::ContentBlock::Text(acp::TextContent::new(prompt))
    ];
    if !context.is_empty() {
        let ctx = context.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(format!("[context]\n{ctx}"))));
    }

    let prompt_started = Instant::now();
    let prompt_result = conn.conn.prompt(acp::PromptRequest::new(session_id.clone(), blocks)).await;

    // Clear delta sender
    if let Ok(mut slot) = conn.delta_slot.lock() { *slot = None; }

    match prompt_result {
        Ok(resp) => {
            acp_log("stage.ok", json!({
                "runtime": runtime_key,
                "stage": "session/prompt",
                "latencyMs": prompt_started.elapsed().as_millis(),
                "stopReason": format!("{:?}", resp.stop_reason)
            }));
            let _ = result_tx.send(Ok((String::new(), session_id.to_string())));
        }
        Err(e) => {
            acp_log("pool.invalidate", json!({ "runtime": runtime_key, "error": e.to_string() }));
            *guard = None;
            let _ = result_tx.send(Err(e.to_string()));
        }
    }
}

async fn handle_prewarm(
    slot: Arc<SlotHandle>,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    role_name: String,
    cwd: String,
) {
    let mut guard = slot.conn.lock().await;
    if guard.is_some() { return; }
    let resolved = resolve_cwd(&cwd);
    acp_log("prewarm.start", json!({ "runtime": runtime_key, "role": role_name }));
    match cold_start(&binary, &args, &env, &resolved).await {
        Ok(conn) => {
            acp_log("prewarm.ok", json!({ "runtime": runtime_key, "role": role_name, "sessionId": conn.session_id.to_string() }));
            *guard = Some(conn);
        }
        Err(e) => {
            acp_log("prewarm.error", json!({ "runtime": runtime_key, "role": role_name, "error": e }));
        }
    }
}

// ── ACP Client impl ───────────────────────────────────────────────────────────

/// Shared between the worker and the ACP client notification handler.
/// Worker sets the sender before each prompt, clears it after.
type DeltaSlot = Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>;

struct UnionAiClient {
    delta_slot: DeltaSlot,
}

#[async_trait::async_trait(?Send)]
impl acp::Client for UnionAiClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        // Auto-approve: select the first option if present
        let outcome = if let Some(opt) = args.options.first() {
            acp::RequestPermissionOutcome::Selected(
                acp::SelectedPermissionOutcome::new(opt.option_id.clone())
            )
        } else {
            acp::RequestPermissionOutcome::Cancelled
        };
        Ok(acp::RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> acp::Result<()> {
        if let acp::SessionUpdate::AgentMessageChunk(chunk) = args.update {
            let text = match chunk.content {
                acp::ContentBlock::Text(tc) => tc.text,
                acp::ContentBlock::ResourceLink(rl) => rl.uri,
                _ => return Ok(()),
            };
            if !text.is_empty() {
                if let Ok(guard) = self.delta_slot.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(text);
                    }
                }
            }
        }
        Ok(())
    }
}

// ── cold_start ────────────────────────────────────────────────────────────────

async fn cold_start(
    binary: &str,
    args: &[String],
    env_pairs: &[(String, String)],
    abs_cwd: &str,
) -> Result<LiveConnection, String> {
    let delta_slot: DeltaSlot = Arc::new(Mutex::new(None));
    acp_log("spawn.start", json!({ "binary": binary, "cwd": abs_cwd }));

    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args)
        .current_dir(abs_cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env_pairs { cmd.env(k, v); }

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().ok_or("stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;

    if let Some(stderr) = child.stderr.take() {
        let bin = binary.to_string();
        tokio::task::spawn_local(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut r = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.trim().is_empty() =>
                        acp_log("stderr", json!({ "binary": bin, "line": clip(line.trim(), 360) })),
                    _ => {}
                }
            }
        });
    }

    acp_log("spawn.ok", json!({ "binary": binary, "pid": child.id() }));

    let (conn, io_future) = acp::ClientSideConnection::new(
        UnionAiClient { delta_slot: delta_slot.clone() },
        stdin.compat_write(),
        stdout.compat(),
        |fut| { tokio::task::spawn_local(fut); },
    );

    let io_handle = tokio::task::spawn_local(async move {
        if let Err(e) = io_future.await {
            acp_log("io_task.error", json!({ "error": e.to_string() }));
        }
    });

    // initialize
    let t = Instant::now();
    conn.initialize(
        acp::InitializeRequest::new(acp::ProtocolVersion::V1)
            .client_info(acp::Implementation::new("unionai", "0.1.0").title("UnionAI")),
    ).await.map_err(|e| e.to_string())?;
    acp_log("stage.ok", json!({ "stage": "initialize", "latencyMs": t.elapsed().as_millis() }));

    // session/new
    let t = Instant::now();
    let resp = conn.new_session(
        acp::NewSessionRequest::new(std::path::PathBuf::from(abs_cwd))
    ).await.map_err(|e| e.to_string())?;
    let session_id = resp.session_id;
    acp_log("stage.ok", json!({ "stage": "session/new", "latencyMs": t.elapsed().as_millis(), "sessionId": session_id.to_string() }));

    Ok(LiveConnection { conn, session_id, cwd: abs_cwd.to_string(), delta_slot, _child: child, _io_task: io_handle })
}

// ── public API ────────────────────────────────────────────────────────────────

pub async fn execute_runtime(
    runtime_kind: &str,
    role_name: &str,
    prompt: &str,
    context: &[(String, String)],
    cwd: &str,
    app: &tauri::AppHandle,
) -> AcpPromptResult {
    let normalized = runtime_kind.trim().to_ascii_lowercase();

    if normalized.is_empty() || normalized == "mock" {
        return mock_execute(role_name, prompt, context);
    }

    let adapter = match build_stdio_adapter(&normalized) {
        Ok(Some(a)) => a,
        Ok(None) => return AcpPromptResult {
            output: format!("unsupported runtime kind: {}", normalized),
            deltas: vec![],
            meta: json!({ "mode": "unsupported-runtime", "runtime": normalized }),
        },
        Err(e) => return AcpPromptResult {
            output: friendly_error_message(&normalized, &e),
            deltas: vec![],
            meta: json!({ "mode": "adapter-unavailable", "runtime": normalized, "error": e }),
        },
    };

    let agent_kind = adapter.kind;
    let started = Instant::now();
    acp_log("execute.start", json!({
        "runtime": adapter.runtime_key,
        "role": role_name,
        "promptSize": prompt.len(),
        "cwd": cwd
    }));

    let (delta_tx, mut delta_rx) = mpsc::unbounded_channel::<String>();
    let (result_tx, mut result_rx) = oneshot::channel();

    let _ = worker_tx().send(WorkerMsg::Execute {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary.clone(),
        args: adapter.args.clone(),
        env: adapter.env.clone(),
        role_name: role_name.to_string(),
        prompt: prompt.to_string(),
        context: context.to_vec(),
        cwd: cwd.to_string(),
        delta_tx,
        result_tx,
    });

    // Stream deltas while waiting for result
    let app = app.clone();
    let role_owned = role_name.to_string();
    let mut deltas: Vec<String> = Vec::new();
    let mut buffer = String::new();
    let mut last_emit = Instant::now();

    // Poll delta_rx and result_rx concurrently
    let result = loop {
        tokio::select! {
            chunk = delta_rx.recv() => {
                match chunk {
                    Some(text) => {
                        buffer.push_str(&text);
                        deltas.push(text.clone());
                        let should_emit = buffer.len() >= 4
                            || text.contains('\n')
                            || last_emit.elapsed() >= std::time::Duration::from_millis(15);
                        if should_emit {
                            let _ = app.emit("acp/delta", json!({
                                "role": role_owned,
                                "delta": buffer.clone()
                            }));
                            buffer.clear();
                            last_emit = Instant::now();
                        }
                    }
                    None => {}
                }
            }
            res = &mut result_rx => {
                // Flush buffer
                if !buffer.is_empty() {
                    let _ = app.emit("acp/delta", json!({ "role": role_owned, "delta": buffer.clone() }));
                }
                // Drain remaining deltas
                while let Ok(text) = delta_rx.try_recv() {
                    deltas.push(text.clone());
                    let _ = app.emit("acp/delta", json!({ "role": role_owned, "delta": text }));
                }
                break res.unwrap_or_else(|_| Err("worker disconnected".to_string()));
            }
        }
    };

    match result {
        Ok((_output, session_id)) => {
            // Output is assembled from deltas (streaming — no separate output field in PromptResponse)
            let output = if deltas.is_empty() {
                format!("{} ACP completed for role {} with no text payload.", adapter.runtime_key, role_name)
            } else {
                deltas.join("")
            };

            acp_log("execute.ok", json!({
                "runtime": adapter.runtime_key,
                "role": role_name,
                "latencyMs": started.elapsed().as_millis(),
                "deltaCount": deltas.len(),
                "outputSize": output.len()
            }));

            AcpPromptResult {
                output,
                deltas,
                meta: json!({
                    "mode": "live",
                    "agentKind": agent_kind,
                    "runtimeKey": adapter.runtime_key,
                    "sessionId": session_id
                }),
            }
        }
        Err(e) => {
            let friendly = friendly_error_message(adapter.runtime_key, &e);
            AcpPromptResult {
                output: friendly.clone(),
                deltas: vec![],
                meta: json!({ "mode": "acp-error", "runtime": adapter.runtime_key, "error": e, "friendlyMessage": friendly }),
            }
        }
    }
}

pub async fn prewarm_role(runtime_kind: &str, role_name: &str, cwd: &str) {
    let adapter = match build_stdio_adapter(runtime_kind) {
        Ok(Some(a)) => a,
        _ => return,
    };
    let _ = worker_tx().send(WorkerMsg::Prewarm {
        runtime_key: adapter.runtime_key,
        binary: adapter.binary,
        args: adapter.args,
        env: adapter.env,
        role_name: role_name.to_string(),
        cwd: cwd.to_string(),
    });
}

pub async fn prewarm(runtime_kind: &str, cwd: &str) {
    prewarm_role(runtime_kind, "UnionAIAssistant", cwd).await;
}

// ── adapter candidates ────────────────────────────────────────────────────────

struct StdioAdapterSpec {
    kind: AgentKind,
    runtime_key: &'static str,
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

fn build_stdio_adapter(runtime: &str) -> Result<Option<StdioAdapterSpec>, String> {
    match runtime {
        "claude" | "claude-code" | "claude-acp" => {
            let (binary, args, env) = resolve_candidate("claude-code", &[
                ("claude-agent-acp", &[][..]),
                ("npx", &["-y", "@zed-industries/claude-agent-acp@0.21.0"][..]),
            ])?;
            Ok(Some(StdioAdapterSpec { kind: AgentKind::ClaudeCode, runtime_key: "claude-code", binary, args, env }))
        }
        "gemini" | "gemini-cli" => {
            let (binary, args, env) = resolve_candidate("gemini-cli", &[
                ("gemini", &["--experimental-acp"][..]),
                ("npx", &["-y", "@google/gemini-cli@0.33.1", "--experimental-acp"][..]),
            ])?;
            Ok(Some(StdioAdapterSpec { kind: AgentKind::GeminiCli, runtime_key: "gemini-cli", binary, args, env }))
        }
        "codex" | "codex-cli" | "codex-acp" => {
            let (binary, args, env) = resolve_candidate("codex-cli", &[
                ("codex-acp", &[][..]),
                ("npx", &["-y", "@zed-industries/codex-acp@0.10.0"][..]),
            ])?;
            Ok(Some(StdioAdapterSpec { kind: AgentKind::CodexCli, runtime_key: "codex-cli", binary, args, env }))
        }
        _ => Ok(None),
    }
}

fn resolve_candidate(
    runtime: &str,
    candidates: &[(&str, &[&str])],
) -> Result<(String, Vec<String>, Vec<(String, String)>), String> {
    let mut missing = Vec::new();
    let mut seen = HashSet::new();
    for (binary, args) in candidates {
        if !seen.insert(*binary) { continue; }
        if let Ok(path) = which(binary) {
            return Ok((path.to_string_lossy().to_string(), args.iter().map(|s| s.to_string()).collect(), vec![]));
        }
        missing.push(*binary);
    }
    Err(format!("{} adapter unavailable, missing: {}", runtime, missing.join(", ")))
}

// ── probe / error helpers ─────────────────────────────────────────────────────

pub fn probe_runtime(runtime_kind: &str) -> Option<(bool, String)> {
    let n = runtime_kind.trim().to_ascii_lowercase();
    if n.is_empty() || n == "mock" { return Some((true, "mock".to_string())); }
    match build_stdio_adapter(&n) {
        Ok(Some(a)) => Some((true, a.binary)),
        Ok(None) => Some((false, format!("unsupported runtime kind: {}", n))),
        Err(e) => Some((false, e)),
    }
}

fn friendly_error_message(runtime: &str, raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    if l.contains("429") || l.contains("rate limit") || l.contains("quota")
        || l.contains("resource_exhausted") || l.contains("too many requests") {
        return format!("[{runtime}] Rate limit exceeded — please wait and retry.");
    }
    if l.contains("epipe") || l.contains("broken pipe") || l.contains("transport closed") {
        return format!("[{runtime}] Agent process exited unexpectedly. It will restart on next message.");
    }
    if l.contains("timeout") {
        return format!("[{runtime}] No response within timeout — please retry.");
    }
    if l.contains("binary not found") || l.contains("adapter unavailable") {
        return format!("[{runtime}] Agent not found. Install: npx -y @zed-industries/{runtime}@latest");
    }
    format!("[{runtime}] {raw}")
}

// ── mock ──────────────────────────────────────────────────────────────────────

fn mock_execute(role: &str, prompt: &str, ctx: &[(String, String)]) -> AcpPromptResult {
    let snap = ctx.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
    let out = format!("{role} prompt: {prompt}. Context: {snap}.");
    AcpPromptResult {
        output: out.clone(),
        deltas: out.as_bytes().chunks(28).map(|c| String::from_utf8_lossy(c).to_string()).collect(),
        meta: json!({ "mode": "mock", "agentKind": AgentKind::Mock, "runtimeKey": "mock" }),
    }
}

// ── utilities ─────────────────────────────────────────────────────────────────

fn resolve_cwd(cwd: &str) -> String {
    let p = std::path::Path::new(cwd);
    let abs = if p.is_absolute() { p.to_path_buf() }
              else { std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf()) };
    abs.canonicalize().unwrap_or(abs).to_string_lossy().to_string()
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn acp_log(event: &str, payload: Value) {
    eprintln!("[unionai.acp] {} {} {}", now_ms(), event, payload);
}
