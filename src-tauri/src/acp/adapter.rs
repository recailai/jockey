use crate::runtime_kind::RuntimeKind;
use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use which::which;

#[derive(Clone)]
struct AdapterResolution {
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    launch_method: String,
}

static ADAPTER_CACHE: OnceLock<DashMap<RuntimeKind, Result<AdapterResolution, String>>> =
    OnceLock::new();
fn adapter_cache() -> &'static DashMap<RuntimeKind, Result<AdapterResolution, String>> {
    ADAPTER_CACHE.get_or_init(DashMap::new)
}

pub fn clear_adapter_cache() {
    adapter_cache().clear();
}

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static ACP_LOG_RING: OnceLock<Mutex<VecDeque<AcpLogEntry>>> = OnceLock::new();
const ACP_LOG_RING_LIMIT: usize = 512;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpLogEntry {
    pub ts_ms: u128,
    pub event: String,
    pub payload: Value,
}

pub fn set_app_data_dir(path: PathBuf) {
    let _ = APP_DATA_DIR.set(path);
}

fn app_data_adapter_bin(binary: &str) -> Option<PathBuf> {
    let base = APP_DATA_DIR.get()?;
    let candidate = base
        .join("adapters")
        .join("node_modules")
        .join(".bin")
        .join(binary);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

pub(super) struct StdioAdapterSpec {
    pub(super) kind: RuntimeKind,
    pub(super) runtime_key: &'static str,
    pub(super) binary: String,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<(String, String)>,
    pub(super) launch_method: String,
}

pub(super) fn build_stdio_adapter(runtime: &str) -> Result<Option<StdioAdapterSpec>, String> {
    let Some(kind) = RuntimeKind::from_str(runtime) else {
        return Ok(None);
    };
    if kind.is_mock() {
        return Ok(None);
    }
    let cached = adapter_cache().get(&kind).map(|r| r.clone());
    let resolved = if let Some(r) = cached {
        r
    } else {
        let r = resolve_adapter_for_kind(kind);
        adapter_cache().insert(kind, r.clone());
        r
    };
    let resolved = resolved?;
    Ok(Some(StdioAdapterSpec {
        kind,
        runtime_key: kind.runtime_key(),
        binary: resolved.binary,
        args: resolved.args,
        env: resolved.env,
        launch_method: resolved.launch_method,
    }))
}

fn resolve_adapter_for_kind(kind: RuntimeKind) -> Result<AdapterResolution, String> {
    match kind {
        RuntimeKind::ClaudeCode => resolve_node_adapter(
            "claude-code",
            "claude-agent-acp",
            "@agentclientprotocol/claude-agent-acp@latest",
            &[],
            None,
        ),
        RuntimeKind::GeminiCli => resolve_node_adapter(
            "gemini-cli",
            "gemini",
            "@google/gemini-cli@latest",
            &["--experimental-acp"],
            Some("--experimental-acp"),
        ),
        RuntimeKind::CodexCli => resolve_node_adapter(
            "codex-cli",
            "codex-acp",
            "@zed-industries/codex-acp@0.12.0",
            &[],
            None,
        ),
        RuntimeKind::Mock => unreachable!(),
    }
}

fn resolve_node_adapter(
    runtime: &str,
    adapter_binary: &str,
    package: &str,
    package_args: &[&str],
    required_help_arg: Option<&str>,
) -> Result<AdapterResolution, String> {
    if let Some(path) = app_data_adapter_bin(adapter_binary) {
        let binary = path.to_string_lossy().to_string();
        if required_help_arg
            .map(|arg| supports_arg_in_help(&binary, arg))
            .unwrap_or(true)
        {
            return Ok(AdapterResolution {
                binary,
                args: package_args.iter().map(|s| s.to_string()).collect(),
                env: vec![],
                launch_method: "managed-binary".to_string(),
            });
        }
    }

    if let Ok(path) = which(adapter_binary) {
        let binary = path.to_string_lossy().to_string();
        if required_help_arg
            .map(|arg| supports_arg_in_help(&binary, arg))
            .unwrap_or(true)
        {
            return Ok(AdapterResolution {
                binary,
                args: package_args.iter().map(|s| s.to_string()).collect(),
                env: vec![],
                launch_method: "path-binary".to_string(),
            });
        }
    }

    let package_candidates = [
        ("pnpm", vec!["dlx", package]),
        ("npx", vec!["-y", package]),
    ];
    let mut missing = vec![adapter_binary.to_string()];
    for (binary, base_args) in package_candidates {
        if let Ok(path) = which(binary) {
            let mut args = base_args.into_iter().map(|s| s.to_string()).collect::<Vec<_>>();
            args.extend(package_args.iter().map(|s| s.to_string()));
            return Ok(AdapterResolution {
                binary: path.to_string_lossy().to_string(),
                args,
                env: vec![],
                launch_method: format!("package-runner:{binary}"),
            });
        }
        missing.push(binary.to_string());
    }

    Err(format!(
        "{} adapter unavailable, missing: {}",
        runtime,
        missing.join(", ")
    ))
}

pub fn adapter_launch_method(runtime_kind: &str) -> Option<String> {
    let n = runtime_kind.trim().to_ascii_lowercase();
    if n.is_empty() || n == "mock" {
        return Some("mock".to_string());
    }
    match build_stdio_adapter(&n) {
        Ok(Some(a)) => Some(a.launch_method),
        _ => None,
    }
}

pub fn probe_runtime(runtime_kind: &str) -> Option<(bool, String)> {
    let n = runtime_kind.trim().to_ascii_lowercase();
    if n.is_empty() || n == "mock" {
        return Some((true, "mock".to_string()));
    }
    match build_stdio_adapter(&n) {
        Ok(Some(a)) => Some((true, format!("{} ({})", a.binary, a.launch_method))),
        Ok(None) => Some((false, format!("unsupported runtime kind: {}", n))),
        Err(e) => Some((false, e)),
    }
}

pub(super) fn friendly_error_message(runtime: &str, raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    if l.contains("429")
        || l.contains("rate limit")
        || l.contains("quota")
        || l.contains("resource_exhausted")
        || l.contains("too many requests")
    {
        return format!("[{runtime}] Rate limit exceeded — please wait and retry.");
    }
    if l.contains("auth_required") {
        return format!("[{runtime}] Authentication required. Log in to the agent CLI and retry.");
    }
    if l.contains("connection_failed") || l.contains("process_crashed") {
        return format!(
            "[{runtime}] Agent process exited unexpectedly. It will restart on next message."
        );
    }
    if l.contains("prompt_timeout") {
        return format!("[{runtime}] Operation timed out — please retry.");
    }
    if l.contains("epipe") || l.contains("broken pipe") || l.contains("transport closed") {
        return format!(
            "[{runtime}] Agent process exited unexpectedly. It will restart on next message."
        );
    }
    if l.contains("timeout") {
        return format!("[{runtime}] Operation timed out — please retry.");
    }
    if l.contains("agent process exited") || l.contains("no longer alive") {
        return format!(
            "[{runtime}] Agent process exited unexpectedly. It will restart on next message."
        );
    }
    if l.contains("model is not supported when using codex with a chatgpt account")
        || (l.contains("not supported") && l.contains("codex") && l.contains("chatgpt account"))
    {
        return format!(
            "[{runtime}] Selected model is incompatible with this Codex account. Clear the model override or choose a supported GPT model."
        );
    }
    if l.contains("missing --experimental-acp")
        || l.contains("unsupported: missing --experimental-acp")
    {
        return format!("[{runtime}] Installed CLI version does not support ACP. Please install a compatible version.");
    }
    if l.contains("/.npm/_npx") && l.contains("enoent") {
        return format!(
            "[{runtime}] npx cache is corrupted. Run: rm -rf ~/.npm/_npx && npm cache verify"
        );
    }
    if l.contains("binary not found") || l.contains("adapter unavailable") {
        let hint = RuntimeKind::from_str(runtime)
            .map(|k| k.install_hint())
            .unwrap_or("");
        if hint.is_empty() {
            return format!("[{runtime}] Agent not found.");
        }
        return format!("[{runtime}] Agent not found. Install with:\n  {hint}");
    }
    format!("[{runtime}] {raw}")
}

pub(super) fn resolve_cwd(cwd: &str) -> String {
    let p = std::path::Path::new(cwd);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    abs.canonicalize()
        .unwrap_or(abs)
        .to_string_lossy()
        .to_string()
}

pub(super) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub(super) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect()
}

fn supports_arg_in_help(binary: &str, arg_flag: &str) -> bool {
    let Ok(output) = std::process::Command::new(binary).arg("--help").output() else {
        return false;
    };
    let arg_lc = arg_flag.to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains(&arg_lc) || stderr.contains(&arg_lc)
}

pub(super) fn acp_log(event: &str, payload: Value) {
    let ts_ms = now_ms();
    if let Ok(mut ring) = ACP_LOG_RING
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
    {
        ring.push_back(AcpLogEntry {
            ts_ms,
            event: event.to_string(),
            payload: payload.clone(),
        });
        while ring.len() > ACP_LOG_RING_LIMIT {
            ring.pop_front();
        }
    }
    eprintln!("[jockey.acp] {} {} {}", ts_ms, event, payload);
}

pub fn acp_log_snapshot(limit: Option<usize>) -> Vec<AcpLogEntry> {
    let max = limit.unwrap_or(ACP_LOG_RING_LIMIT).min(ACP_LOG_RING_LIMIT);
    let mut out = ACP_LOG_RING
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map(|ring| ring.iter().rev().take(max).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    out.reverse();
    out
}
