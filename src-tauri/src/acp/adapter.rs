use crate::runtime_kind::RuntimeKind;
use crate::runtime_profile::{self, RuntimeProfile};
use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use which::which;

const CLAUDE_ACP_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@0.70.0";

#[derive(Clone)]
struct AdapterResolution {
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    launch_method: String,
    transport: AdapterTransport,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeProtocol {
    CodexAppServer,
    PiRpc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadlessProtocol {
    AgyStreamJson,
    ClaudeStreamJson,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterTransport {
    Acp,
    Native(NativeProtocol),
    HeadlessJson {
        protocol: HeadlessProtocol,
        stream_input: bool,
        output_format: bool,
        conversation: bool,
    },
}

static ADAPTER_CACHE: OnceLock<DashMap<RuntimeKind, Result<AdapterResolution, String>>> =
    OnceLock::new();
fn adapter_cache() -> &'static DashMap<RuntimeKind, Result<AdapterResolution, String>> {
    ADAPTER_CACHE.get_or_init(DashMap::new)
}

pub fn clear_adapter_cache() {
    adapter_cache().clear();
    if let Some(cache) = SHELL_ENV_CACHE.get() {
        if let Ok(mut env) = cache.lock() {
            *env = None;
        }
    }
}

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// App data dir for components outside the adapter module (permission bridge
/// config files). Returns None before app setup has run.
pub(crate) fn app_data_dir() -> Option<&'static std::path::Path> {
    APP_DATA_DIR.get().map(|p| p.as_path())
}
static SHELL_ENV_CACHE: OnceLock<Mutex<Option<Vec<(String, String)>>>> = OnceLock::new();
static ACP_LOG_RING: OnceLock<Mutex<VecDeque<AcpLogEntry>>> = OnceLock::new();
const ACP_LOG_RING_LIMIT: usize = 512;
const AGENT_ENV_ALLOWLIST: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
];

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

pub(crate) struct StdioAdapterSpec {
    pub(crate) kind: RuntimeKind,
    pub(crate) runtime_key: &'static str,
    pub(crate) binary: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) launch_method: String,
    pub(crate) transport: AdapterTransport,
}

pub(super) fn build_stdio_adapter(runtime: &str) -> Result<Option<StdioAdapterSpec>, String> {
    let normalized = runtime.trim().to_ascii_lowercase();
    if let Some(profile) = runtime_profile::custom_profile(&normalized) {
        return build_custom_stdio_adapter(profile);
    }
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
        transport: resolved.transport,
    }))
}

fn build_custom_stdio_adapter(profile: RuntimeProfile) -> Result<Option<StdioAdapterSpec>, String> {
    let launch = profile
        .launch_spec
        .clone()
        .ok_or_else(|| format!("custom ACP profile has no launch spec: {}", profile.id))?;
    let binary = find_binary(&launch.command)
        .or_else(|| {
            let path = PathBuf::from(&launch.command);
            path.is_file().then_some(path)
        })
        .ok_or_else(|| {
            format!(
                "custom ACP profile '{}' unavailable, missing executable: {}",
                profile.label, launch.command
            )
        })?
        .to_string_lossy()
        .to_string();
    let declared_env = launch
        .env_refs
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut env = shell_environment()
        .into_iter()
        .filter(|(key, value)| {
            !value.trim().is_empty() && (key == "PATH" || declared_env.contains(key.as_str()))
        })
        .collect::<Vec<_>>();
    for key in &launch.env_refs {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() && !env.iter().any(|(known, _)| known == key) {
                env.push((key.clone(), value));
            }
        }
    }
    let runtime_key = runtime_profile::runtime_key_static(&profile.id)
        .ok_or_else(|| format!("custom ACP profile not registered: {}", profile.id))?;
    Ok(Some(StdioAdapterSpec {
        kind: RuntimeKind::ClaudeCode,
        runtime_key,
        binary,
        args: launch.args,
        env,
        launch_method: "custom-acp-profile".to_string(),
        transport: AdapterTransport::Acp,
    }))
}

fn resolve_adapter_for_kind(kind: RuntimeKind) -> Result<AdapterResolution, String> {
    match kind {
        RuntimeKind::ClaudeNative => resolve_headless_adapter(HeadlessProtocol::ClaudeStreamJson),
        RuntimeKind::ClaudeCode => resolve_node_adapter(
            "claude-code",
            "claude-agent-acp",
            CLAUDE_ACP_PACKAGE,
            &[],
            None,
            None,
            false,
        ),
        RuntimeKind::AntigravityCli => resolve_headless_adapter(HeadlessProtocol::AgyStreamJson),
        RuntimeKind::CodexCli => resolve_native_adapter(
            "codex-cli",
            "codex",
            vec!["app-server"],
            NativeProtocol::CodexAppServer,
            Some("app-server"),
        ),
        RuntimeKind::PiCli => resolve_native_adapter(
            "pi-cli",
            "pi",
            vec!["--mode", "rpc"],
            NativeProtocol::PiRpc,
            Some("--mode"),
        ),
        RuntimeKind::Mock => unreachable!(),
    }
}

fn resolve_native_adapter(
    runtime: &str,
    binary_name: &str,
    args: Vec<&str>,
    protocol: NativeProtocol,
    required_help_arg: Option<&str>,
) -> Result<AdapterResolution, String> {
    let path = find_binary(binary_name).ok_or_else(|| {
        format!("{runtime} native adapter unavailable, missing executable: {binary_name}")
    })?;
    let binary = path.to_string_lossy().to_string();
    if required_help_arg.is_some_and(|arg| !supports_arg_in_help(&binary, arg)) {
        return Err(format!(
            "{runtime} native adapter unavailable, installed {binary_name} does not support the required native mode"
        ));
    }
    let launch_method = match protocol {
        NativeProtocol::CodexAppServer => "native:codex-app-server",
        NativeProtocol::PiRpc => "native:pi-rpc",
    };
    Ok(AdapterResolution {
        binary,
        args: args.into_iter().map(str::to_string).collect(),
        env: agent_env_overrides(),
        launch_method: launch_method.to_string(),
        transport: AdapterTransport::Native(protocol),
    })
}

fn resolve_node_adapter(
    runtime: &str,
    adapter_binary: &str,
    package: &str,
    package_args: &[&str],
    required_help_arg: Option<&str>,
    required_probe_contains: Option<(&str, &str)>,
    prefer_package: bool,
) -> Result<AdapterResolution, String> {
    if let Some(path) = app_data_adapter_bin(adapter_binary) {
        let binary = path.to_string_lossy().to_string();
        if required_help_arg
            .map(|arg| supports_arg_in_help(&binary, arg))
            .unwrap_or(true)
            && required_probe_contains
                .map(|(arg, needle)| command_output_contains(&binary, arg, needle))
                .unwrap_or(true)
        {
            return Ok(AdapterResolution {
                binary,
                args: package_args.iter().map(|s| s.to_string()).collect(),
                env: agent_env_overrides(),
                launch_method: "managed-binary".to_string(),
                transport: AdapterTransport::Acp,
            });
        }
    }

    if !prefer_package {
        if let Some(resolved) = resolve_path_adapter(
            adapter_binary,
            package_args,
            required_help_arg,
            required_probe_contains,
        ) {
            return Ok(resolved);
        }
    }

    // Controlled install: pin the package into the app data dir instead of
    // relying on an ephemeral package-runner cache for production launches.
    if ensure_managed_bridge_install(package, adapter_binary) {
        if let Some(path) = app_data_adapter_bin(adapter_binary) {
            let binary = path.to_string_lossy().to_string();
            if required_help_arg
                .map(|arg| supports_arg_in_help(&binary, arg))
                .unwrap_or(true)
                && required_probe_contains
                    .map(|(arg, needle)| command_output_contains(&binary, arg, needle))
                    .unwrap_or(true)
            {
                return Ok(AdapterResolution {
                    binary,
                    args: package_args.iter().map(|s| s.to_string()).collect(),
                    env: agent_env_overrides(),
                    launch_method: "managed-install".to_string(),
                    transport: AdapterTransport::Acp,
                });
            }
        }
    }

    // Package-runner fallback: only for development builds or when explicitly
    // enabled for diagnostics. Never used as a silent production path.
    let package_runner_allowed = cfg!(debug_assertions)
        || std::env::var("JOCKEY_ALLOW_PACKAGE_RUNNER").as_deref() == Ok("1");
    if package_runner_allowed {
        let package_candidates = [("pnpm", vec!["dlx", package]), ("npx", vec!["-y", package])];
        for (binary, base_args) in package_candidates {
            if let Some(path) = find_binary(binary) {
                let mut args = base_args
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                args.extend(package_args.iter().map(|s| s.to_string()));
                return Ok(AdapterResolution {
                    binary: path.to_string_lossy().to_string(),
                    args,
                    env: agent_env_overrides(),
                    launch_method: format!("package-runner:{binary}"),
                    transport: AdapterTransport::Acp,
                });
            }
        }
    }

    if prefer_package {
        if let Some(resolved) = resolve_path_adapter(
            adapter_binary,
            package_args,
            required_help_arg,
            required_probe_contains,
        ) {
            return Ok(resolved);
        }
    }

    Err(format!(
        "{} adapter unavailable, missing: {}{}",
        runtime,
        adapter_binary,
        if package_runner_allowed {
            ", pnpm, npx".to_string()
        } else {
            format!(
                "; managed install into {} failed or was skipped (set JOCKEY_ALLOW_PACKAGE_RUNNER=1 to permit package-runner fallback)",
                APP_DATA_DIR
                    .get()
                    .map(|d| d.join("adapters").to_string_lossy().to_string())
                    .unwrap_or_default(),
            )
        },
    ))
}

/// Install the bridge package into `<app_data_dir>/adapters` once per app
/// run. The install is recorded in a manifest recording package, timestamp
/// and npm version so a later run can detect a stale or foreign install.
/// Returns true when `app_data_adapter_bin` can be expected to resolve.
fn ensure_managed_bridge_install(package: &str, adapter_binary: &str) -> bool {
    static INSTALL_ATTEMPTED: OnceLock<()> = OnceLock::new();
    if app_data_adapter_bin(adapter_binary).is_some() {
        return true;
    }
    let Some(base) = APP_DATA_DIR.get() else {
        return false;
    };
    INSTALL_ATTEMPTED.get_or_init(|| {
        let dir = base.join("adapters");
        let manifest_path = dir.join("bridge-install.json");
        if manifest_path.exists() {
            // A manifest exists but the binary is missing: the install is
            // broken. Reinstall once.
            acp_log(
                "adapter.bridge.reinstall",
                serde_json::json!({ "package": package }),
            );
        }
        let Some(npm) = find_binary("npm") else {
            acp_log(
                "adapter.bridge.install.skipped",
                serde_json::json!({ "reason": "npm not found" }),
            );
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let output = std::process::Command::new(npm)
            .args([
                "install",
                "--prefix",
                &dir.to_string_lossy(),
                "--no-audit",
                "--no-fund",
                "--loglevel",
                "error",
                package,
            ])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let manifest = serde_json::json!({
                    "package": package,
                    "installedAt": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or_default(),
                    "npmVersion": detect_npm_version(),
                });
                if let Ok(text) = serde_json::to_string_pretty(&manifest) {
                    let _ = std::fs::write(&manifest_path, text);
                }
                acp_log(
                    "adapter.bridge.installed",
                    serde_json::json!({ "package": package, "dir": dir.to_string_lossy() }),
                );
            }
            other => {
                acp_log(
                    "adapter.bridge.install.failed",
                    serde_json::json!({
                        "package": package,
                        "status": other.map(|o| o.status.to_string()).unwrap_or_default(),
                    }),
                );
            }
        }
    });
    app_data_adapter_bin(adapter_binary).is_some()
}

fn detect_npm_version() -> Option<String> {
    let npm = find_binary("npm")?;
    let output = std::process::Command::new(npm)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn resolve_path_adapter(
    adapter_binary: &str,
    package_args: &[&str],
    required_help_arg: Option<&str>,
    required_probe_contains: Option<(&str, &str)>,
) -> Option<AdapterResolution> {
    let path = find_binary(adapter_binary)?;
    let binary = path.to_string_lossy().to_string();
    if !required_help_arg
        .map(|arg| supports_arg_in_help(&binary, arg))
        .unwrap_or(true)
    {
        return None;
    }
    if !required_probe_contains
        .map(|(arg, needle)| command_output_contains(&binary, arg, needle))
        .unwrap_or(true)
    {
        return None;
    }
    Some(AdapterResolution {
        binary,
        args: package_args.iter().map(|s| s.to_string()).collect(),
        env: agent_env_overrides(),
        launch_method: "path-binary".to_string(),
        transport: AdapterTransport::Acp,
    })
}

fn resolve_headless_adapter(protocol: HeadlessProtocol) -> Result<AdapterResolution, String> {
    let (runtime, binary_name) = match protocol {
        HeadlessProtocol::AgyStreamJson => ("antigravity-cli", "agy"),
        HeadlessProtocol::ClaudeStreamJson => ("claude-native", "claude"),
    };
    let path = find_binary(binary_name)
        .ok_or_else(|| format!("{runtime} adapter unavailable, missing: {binary_name}"))?;
    let binary = path.to_string_lossy().to_string();
    if !supports_arg_in_help(&binary, "--print") {
        return Err(format!(
            "{runtime} adapter unavailable, installed {binary_name} does not support --print"
        ));
    }
    if matches!(protocol, HeadlessProtocol::ClaudeStreamJson)
        && (!supports_arg_in_help(&binary, "--resume")
            || !supports_arg_in_help(&binary, "--permission-prompt-tool"))
    {
        return Err(
            "claude-native adapter unavailable, installed claude lacks stream resume or permission handler support"
                .to_string(),
        );
    }
    if matches!(protocol, HeadlessProtocol::ClaudeStreamJson)
        && !super::session::perm_bridge::is_ready()
    {
        return Err(
            "claude-native adapter unavailable, Jockey permission bridge is not ready".to_string(),
        );
    }
    let output_format = supports_arg_in_help(&binary, "--output-format");
    let stream_input = supports_arg_in_help(&binary, "--input-format");
    let conversation = match protocol {
        HeadlessProtocol::AgyStreamJson => supports_arg_in_help(&binary, "--conversation"),
        HeadlessProtocol::ClaudeStreamJson => supports_arg_in_help(&binary, "--resume"),
    };
    if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) && (!output_format || !stream_input) {
        return Err(
            "claude-native adapter unavailable, installed claude lacks stream-json input/output support"
                .to_string(),
        );
    }
    Ok(AdapterResolution {
        binary,
        args: if matches!(protocol, HeadlessProtocol::ClaudeStreamJson) {
            vec!["-p".to_string(), "--include-partial-messages".to_string()]
        } else {
            Vec::new()
        },
        env: agent_env_overrides(),
        launch_method: match protocol {
            HeadlessProtocol::AgyStreamJson => "headless-binary".to_string(),
            HeadlessProtocol::ClaudeStreamJson => "native:claude-stream-json".to_string(),
        },
        transport: AdapterTransport::HeadlessJson {
            protocol,
            stream_input,
            output_format,
            conversation,
        },
    })
}

fn find_binary(binary: &str) -> Option<PathBuf> {
    which(binary).ok().or_else(|| {
        let path = shell_path()?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

fn shell_path() -> Option<String> {
    shell_environment()
        .iter()
        .find_map(|(key, value)| (key == "PATH").then_some(value))
        .cloned()
}

fn agent_env_overrides() -> Vec<(String, String)> {
    let shell_env = shell_environment();
    let out = shell_env
        .iter()
        .filter(|(key, value)| {
            !value.trim().is_empty()
                && if key == "PATH" {
                    std::env::var("PATH")
                        .map(|existing| existing != *value)
                        .unwrap_or(true)
                } else {
                    AGENT_ENV_ALLOWLIST.contains(&key.as_str())
                        && !std::env::var(key)
                            .map(|existing| !existing.trim().is_empty())
                            .unwrap_or(false)
                }
        })
        .cloned()
        .collect::<Vec<_>>();
    if !out.is_empty() {
        acp_log(
            "adapter.shell_env.loaded",
            serde_json::json!({ "keys": out.iter().map(|(k, _)| k).collect::<Vec<_>>() }),
        );
    }
    out
}

fn shell_environment() -> Vec<(String, String)> {
    let cache = SHELL_ENV_CACHE.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = cache.lock() else {
        return load_interactive_shell_env();
    };
    if guard.is_none() {
        *guard = Some(load_interactive_shell_env());
    }
    guard.as_ref().cloned().unwrap_or_default()
}

#[cfg(unix)]
fn load_interactive_shell_env() -> Vec<(String, String)> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = std::process::Command::new(shell)
        .arg("-lic")
        .arg("env")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(not(unix))]
fn load_interactive_shell_env() -> Vec<(String, String)> {
    Vec::new()
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

pub fn adapter_transport(runtime_kind: &str) -> Option<String> {
    let normalized = RuntimeKind::from_str(runtime_kind)
        .map(|kind| kind.runtime_key().to_string())
        .unwrap_or_else(|| runtime_kind.trim().to_ascii_lowercase());
    match build_stdio_adapter(&normalized) {
        Ok(Some(adapter)) => Some(match adapter.transport {
            AdapterTransport::Acp => "acp".to_string(),
            AdapterTransport::Native(protocol) => native_protocol_name(protocol).to_string(),
            AdapterTransport::HeadlessJson { protocol, .. } => match protocol {
                HeadlessProtocol::AgyStreamJson => "agy-stream-json".to_string(),
                HeadlessProtocol::ClaudeStreamJson => "claude-stream-json".to_string(),
            },
        }),
        _ => None,
    }
}

fn native_protocol_name(protocol: NativeProtocol) -> &'static str {
    match protocol {
        NativeProtocol::CodexAppServer => "native-codex-app-server",
        NativeProtocol::PiRpc => "native-pi-rpc",
    }
}

/// Launch details for a resolved adapter, safe to consume outside the `acp`
/// module (provider session administration, prewarm plumbing, etc.).
pub struct ResolvedAdapterLaunch {
    pub binary: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn resolve_adapter_launch(runtime_kind: &str) -> Result<ResolvedAdapterLaunch, String> {
    match build_stdio_adapter(runtime_kind) {
        Ok(Some(spec)) => Ok(ResolvedAdapterLaunch {
            binary: spec.binary,
            args: spec.args,
            env: spec.env,
        }),
        Ok(None) => Err(format!("unsupported runtime kind: {runtime_kind}")),
        Err(error) => Err(error),
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
    if l.contains("process has exited")
        || l.contains("process exited")
        || l.contains("exit code")
        || l.contains("exit status")
    {
        return format!(
            "[{runtime}] Agent process exited unexpectedly. It will restart on next message. Details: {raw}"
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

fn command_output_contains(binary: &str, arg: &str, needle: &str) -> bool {
    let Ok(output) = std::process::Command::new(binary).arg(arg).output() else {
        return false;
    };
    let needle_lc = needle.to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains(&needle_lc) || stderr.contains(&needle_lc)
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
