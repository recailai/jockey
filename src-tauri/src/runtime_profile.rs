use crate::runtime_kind::RuntimeKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeFamily {
    Native,
    Acp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCapabilities {
    pub(crate) streaming: bool,
    pub(crate) session_persistence: bool,
    pub(crate) session_resume: bool,
    pub(crate) model_catalog: bool,
    pub(crate) dynamic_modes: bool,
    pub(crate) dynamic_config: bool,
    pub(crate) permission_requests: bool,
    pub(crate) mcp_servers: bool,
    pub(crate) attachments: bool,
    pub(crate) tool_invocations: bool,
    pub(crate) session_listing: bool,
    pub(crate) rewind: bool,
    pub(crate) fork: bool,
}

impl RuntimeCapabilities {
    pub(crate) fn unavailable() -> Self {
        Self {
            streaming: false,
            session_persistence: false,
            session_resume: false,
            model_catalog: false,
            dynamic_modes: false,
            dynamic_config: false,
            permission_requests: false,
            mcp_servers: false,
            attachments: false,
            tool_invocations: false,
            session_listing: false,
            rewind: false,
            fork: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeLaunchSpec {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env_refs: Vec<String>,
    pub(crate) cwd_strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeProfile {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) family: RuntimeFamily,
    pub(crate) transport: String,
    pub(crate) runtime_key: String,
    pub(crate) capabilities: RuntimeCapabilities,
    pub(crate) launch_spec: Option<RuntimeLaunchSpec>,
    pub(crate) version_requirement: Option<String>,
    pub(crate) update_strategy: String,
    pub(crate) builtin: bool,
}

fn custom_profiles() -> &'static RwLock<HashMap<String, RuntimeProfile>> {
    static PROFILES: OnceLock<RwLock<HashMap<String, RuntimeProfile>>> = OnceLock::new();
    PROFILES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn custom_runtime_keys() -> &'static RwLock<HashMap<String, &'static str>> {
    static KEYS: OnceLock<RwLock<HashMap<String, &'static str>>> = OnceLock::new();
    KEYS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(crate) fn runtime_key_static(profile: &str) -> Option<&'static str> {
    if let Some(kind) = RuntimeKind::from_str(profile) {
        return Some(kind.runtime_key());
    }
    if custom_profile(profile).is_none() {
        return None;
    }
    if let Ok(keys) = custom_runtime_keys().read() {
        if let Some(key) = keys.get(profile) {
            return Some(*key);
        }
    }
    let leaked: &'static str = Box::leak(profile.trim().to_string().into_boxed_str());
    if let Ok(mut keys) = custom_runtime_keys().write() {
        keys.entry(profile.trim().to_string()).or_insert(leaked);
        return keys.get(profile.trim()).copied();
    }
    Some(leaked)
}

pub(crate) fn register_custom_profile(profile: RuntimeProfile) -> Result<(), String> {
    if !profile.id.starts_with("acp:custom:") {
        return Err("custom ACP profile id must start with acp:custom:".to_string());
    }
    if profile.label.trim().is_empty() {
        return Err("custom ACP profile label required".to_string());
    }
    if profile.family != RuntimeFamily::Acp {
        return Err("custom profile family must be ACP".to_string());
    }
    let Some(launch) = profile.launch_spec.as_ref() else {
        return Err("custom ACP profile launch spec required".to_string());
    };
    if launch.command.trim().is_empty() {
        return Err("custom ACP command required".to_string());
    }
    if let Ok(mut profiles) = custom_profiles().write() {
        profiles.insert(profile.id.clone(), profile);
        Ok(())
    } else {
        Err("custom ACP profile registry unavailable".to_string())
    }
}

pub(crate) fn remove_custom_profile(id: &str) {
    if let Ok(mut profiles) = custom_profiles().write() {
        profiles.remove(id);
    }
}

pub(crate) fn custom_profile(id: &str) -> Option<RuntimeProfile> {
    custom_profiles().read().ok()?.get(id).cloned()
}

pub(crate) fn all_profiles() -> Vec<RuntimeProfile> {
    let mut profiles = builtin_profiles();
    if let Ok(custom) = custom_profiles().read() {
        profiles.extend(custom.values().cloned());
    }
    profiles
}

pub(crate) fn profile_id(runtime: &str) -> String {
    match RuntimeKind::from_str(runtime) {
        Some(RuntimeKind::Mock) => "mock".to_string(),
        Some(RuntimeKind::ClaudeCode) => "acp:claude-code".to_string(),
        Some(RuntimeKind::ClaudeNative) => "native:claude".to_string(),
        Some(RuntimeKind::AntigravityCli) => "native:agy".to_string(),
        Some(RuntimeKind::CodexCli) => "native:codex".to_string(),
        Some(RuntimeKind::PiCli) => "native:pi".to_string(),
        None => runtime.trim().to_string(),
    }
}

pub(crate) fn runtime_key(profile: &str) -> String {
    RuntimeKind::from_str(profile)
        .map(|kind| kind.runtime_key().to_string())
        .unwrap_or_else(|| profile.trim().to_string())
}

pub(crate) fn builtin_profiles() -> Vec<RuntimeProfile> {
    vec![
        RuntimeProfile {
            id: "native:claude".to_string(),
            label: "Claude Code".to_string(),
            family: RuntimeFamily::Native,
            transport: "claude-stream-json".to_string(),
            runtime_key: "claude-native".to_string(),
            capabilities: RuntimeCapabilities {
                streaming: true,
                session_persistence: true,
                session_resume: true,
                model_catalog: false,
                dynamic_modes: false,
                dynamic_config: false,
                permission_requests: true,
                mcp_servers: true,
                attachments: false,
                tool_invocations: true,
                session_listing: false,
                rewind: false,
                fork: false,
            },
            launch_spec: None,
            version_requirement: None,
            update_strategy: "managed-cli".to_string(),
            builtin: true,
        },
        RuntimeProfile {
            id: "native:codex".to_string(),
            label: "Codex CLI".to_string(),
            family: RuntimeFamily::Native,
            transport: "codex-app-server".to_string(),
            runtime_key: "codex-cli".to_string(),
            capabilities: RuntimeCapabilities {
                streaming: true,
                session_persistence: true,
                session_resume: true,
                model_catalog: true,
                dynamic_modes: false,
                dynamic_config: true,
                permission_requests: false,
                mcp_servers: false,
                attachments: false,
                tool_invocations: true,
                session_listing: false,
                rewind: false,
                fork: false,
            },
            launch_spec: None,
            version_requirement: None,
            update_strategy: "managed-cli".to_string(),
            builtin: true,
        },
        RuntimeProfile {
            id: "native:pi".to_string(),
            label: "Pi CLI".to_string(),
            family: RuntimeFamily::Native,
            transport: "pi-rpc".to_string(),
            runtime_key: "pi-cli".to_string(),
            capabilities: RuntimeCapabilities {
                streaming: true,
                session_persistence: true,
                session_resume: true,
                model_catalog: true,
                dynamic_modes: false,
                dynamic_config: true,
                permission_requests: false,
                mcp_servers: false,
                attachments: true,
                tool_invocations: true,
                session_listing: false,
                rewind: false,
                fork: false,
            },
            launch_spec: None,
            version_requirement: None,
            update_strategy: "managed-cli".to_string(),
            builtin: true,
        },
        RuntimeProfile {
            id: "native:agy".to_string(),
            label: "Antigravity CLI (agy)".to_string(),
            family: RuntimeFamily::Native,
            transport: "agy-stream-json".to_string(),
            runtime_key: "antigravity-cli".to_string(),
            capabilities: RuntimeCapabilities {
                streaming: true,
                session_persistence: true,
                session_resume: true,
                model_catalog: false,
                dynamic_modes: false,
                dynamic_config: false,
                permission_requests: false,
                mcp_servers: false,
                attachments: false,
                tool_invocations: true,
                session_listing: false,
                rewind: false,
                fork: false,
            },
            launch_spec: None,
            version_requirement: None,
            update_strategy: "managed-cli".to_string(),
            builtin: true,
        },
        RuntimeProfile {
            id: "acp:claude-code".to_string(),
            label: "Claude Code (ACP)".to_string(),
            family: RuntimeFamily::Acp,
            transport: "acp".to_string(),
            runtime_key: "claude-code".to_string(),
            capabilities: RuntimeCapabilities {
                streaming: true,
                session_persistence: true,
                session_resume: true,
                model_catalog: true,
                dynamic_modes: true,
                dynamic_config: true,
                permission_requests: true,
                mcp_servers: true,
                attachments: true,
                tool_invocations: true,
                session_listing: false,
                rewind: false,
                fork: false,
            },
            launch_spec: None,
            version_requirement: Some("@agentclientprotocol/claude-agent-acp@0.70.0".to_string()),
            update_strategy: "pinned-bridge".to_string(),
            builtin: true,
        },
    ]
}
