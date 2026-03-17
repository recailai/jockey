use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use which::which;

use super::worker::AgentKind;

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

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
    pub(super) kind: AgentKind,
    pub(super) runtime_key: &'static str,
    pub(super) binary: String,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<(String, String)>,
}

pub(super) fn build_stdio_adapter(runtime: &str) -> Result<Option<StdioAdapterSpec>, String> {
    match runtime {
        "claude" | "claude-code" | "claude-acp" => {
            let (binary, args, env) = resolve_candidate(
                "claude-code",
                &[
                    ("claude-agent-acp", &[][..]),
                    (
                        "pnpm",
                        &["dlx", "@zed-industries/claude-agent-acp@latest"][..],
                    ),
                    (
                        "npx",
                        &["-y", "@zed-industries/claude-agent-acp@latest"][..],
                    ),
                ],
            )?;
            Ok(Some(StdioAdapterSpec {
                kind: AgentKind::ClaudeCode,
                runtime_key: "claude-code",
                binary,
                args,
                env,
            }))
        }
        "gemini" | "gemini-cli" => {
            if let Ok(path) = which("gemini") {
                let binary = path.to_string_lossy().to_string();
                if supports_arg_in_help(&binary, "--experimental-acp") {
                    return Ok(Some(StdioAdapterSpec {
                        kind: AgentKind::GeminiCli,
                        runtime_key: "gemini-cli",
                        binary,
                        args: vec!["--experimental-acp".to_string()],
                        env: vec![],
                    }));
                }
                return Err(
                    "gemini-cli installed but unsupported: missing --experimental-acp".to_string(),
                );
            }
            let (binary, args, env) = resolve_candidate(
                "gemini-cli",
                &[
                    (
                        "pnpm",
                        &["dlx", "@google/gemini-cli@latest", "--experimental-acp"][..],
                    ),
                    (
                        "npx",
                        &["-y", "@google/gemini-cli@latest", "--experimental-acp"][..],
                    ),
                ],
            )?;
            Ok(Some(StdioAdapterSpec {
                kind: AgentKind::GeminiCli,
                runtime_key: "gemini-cli",
                binary,
                args,
                env,
            }))
        }
        "codex" | "codex-cli" | "codex-acp" => {
            let (binary, args, env) = resolve_candidate(
                "codex-cli",
                &[
                    ("codex-acp", &[][..]),
                    ("pnpm", &["dlx", "@zed-industries/codex-acp@latest"][..]),
                    ("npx", &["-y", "@zed-industries/codex-acp@latest"][..]),
                ],
            )?;
            Ok(Some(StdioAdapterSpec {
                kind: AgentKind::CodexCli,
                runtime_key: "codex-cli",
                binary,
                args,
                env,
            }))
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
        if !seen.insert(*binary) {
            continue;
        }
        if let Some(app_data_path) = app_data_adapter_bin(binary) {
            return Ok((
                app_data_path.to_string_lossy().to_string(),
                args.iter().map(|s| s.to_string()).collect(),
                vec![],
            ));
        }
        if let Ok(path) = which(binary) {
            return Ok((
                path.to_string_lossy().to_string(),
                args.iter().map(|s| s.to_string()).collect(),
                vec![],
            ));
        }
        missing.push(*binary);
    }
    Err(format!(
        "{} adapter unavailable, missing: {}",
        runtime,
        missing.join(", ")
    ))
}

pub fn probe_runtime(runtime_kind: &str) -> Option<(bool, String)> {
    let n = runtime_kind.trim().to_ascii_lowercase();
    if n.is_empty() || n == "mock" {
        return Some((true, "mock".to_string()));
    }
    match build_stdio_adapter(&n) {
        Ok(Some(a)) => Some((true, a.binary)),
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
    if l.contains("epipe") || l.contains("broken pipe") || l.contains("transport closed") {
        return format!(
            "[{runtime}] Agent process exited unexpectedly. It will restart on next message."
        );
    }
    if l.contains("timeout") {
        return format!("[{runtime}] No response within timeout — please retry.");
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
        return format!(
            "[{runtime}] Agent not found. Install: npx -y @zed-industries/{runtime}@latest"
        );
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
    s.chars().take(n).collect()
}

fn supports_arg_in_help(binary: &str, arg_flag: &str) -> bool {
    let output = match std::process::Command::new(binary).arg("--help").output() {
        Ok(out) => out,
        Err(_) => return false,
    };
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout).to_ascii_lowercase());
    text.push_str(&String::from_utf8_lossy(&output.stderr).to_ascii_lowercase());
    text.contains(&arg_flag.to_ascii_lowercase())
}

pub(super) fn acp_log(event: &str, payload: Value) {
    eprintln!("[unionai.acp] {} {} {}", now_ms(), event, payload);
}
