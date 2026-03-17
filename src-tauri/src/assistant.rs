use crate::acp;
use crate::types::*;
use std::process::Command;
use std::sync::OnceLock;

pub(crate) fn detect_binary_version(binary: &str) -> Option<String> {
    match Command::new(binary).arg("--version").output() {
        Ok(output) => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            text.lines().next().map(|s| s.trim().to_string())
        }
        Err(_) => None,
    }
}

static ASSISTANT_CATALOG: OnceLock<Vec<AssistantRuntime>> = OnceLock::new();

pub(crate) fn assistant_catalog() -> &'static Vec<AssistantRuntime> {
    ASSISTANT_CATALOG.get_or_init(|| build_assistant_catalog())
}

pub(crate) fn build_assistant_catalog() -> Vec<AssistantRuntime> {
    let (claude_ok, claude_bin) = acp::probe_runtime("claude-code")
        .unwrap_or((false, "claude-code adapter unavailable".to_string()));
    let (gemini_ok, gemini_bin) = acp::probe_runtime("gemini-cli")
        .unwrap_or((false, "gemini-cli adapter unavailable".to_string()));
    let (codex_ok, codex_bin) = acp::probe_runtime("codex-cli")
        .unwrap_or((false, "codex-cli adapter unavailable".to_string()));
    let claude_v = if claude_ok && claude_bin != "npx" {
        detect_binary_version(&claude_bin)
    } else {
        None
    };
    let gemini_v = if gemini_ok && gemini_bin != "npx" {
        detect_binary_version(&gemini_bin)
    } else {
        None
    };
    let codex_v = if codex_ok && codex_bin != "npx" {
        detect_binary_version(&codex_bin)
    } else {
        None
    };
    vec![
        AssistantRuntime {
            key: "claude-code".to_string(),
            label: "Claude Code".to_string(),
            binary: claude_bin.clone(),
            available: claude_ok,
            version: claude_v,
        },
        AssistantRuntime {
            key: "gemini-cli".to_string(),
            label: "Gemini CLI".to_string(),
            binary: gemini_bin.clone(),
            available: gemini_ok,
            version: gemini_v,
        },
        AssistantRuntime {
            key: "codex-cli".to_string(),
            label: "Codex CLI".to_string(),
            binary: codex_bin.clone(),
            available: codex_ok,
            version: codex_v,
        },
    ]
}

pub(crate) fn normalize_runtime_key(runtime: &str) -> Option<&'static str> {
    match runtime.trim().to_ascii_lowercase().as_str() {
        "gemini" | "gemini-cli" => Some("gemini-cli"),
        "claude" | "claude-code" | "claude-acp" => Some("claude-code"),
        "codex" | "codex-cli" | "codex-acp" => Some("codex-cli"),
        "mock" => Some("mock"),
        _ => None,
    }
}

#[tauri::command]
pub(crate) fn detect_assistants() -> Result<Vec<AssistantRuntime>, String> {
    Ok(assistant_catalog().clone())
}
