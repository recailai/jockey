use crate::acp;
use crate::runtime_kind::RuntimeKind;
use crate::types::*;
use std::process::Command;
use std::sync::OnceLock;

pub(crate) fn detect_binary_version(binary: &str) -> Option<String> {
    match Command::new(binary).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

static ASSISTANT_CATALOG: OnceLock<Vec<AssistantRuntime>> = OnceLock::new();

pub(crate) fn assistant_catalog() -> &'static Vec<AssistantRuntime> {
    ASSISTANT_CATALOG.get_or_init(|| build_assistant_catalog())
}

pub(crate) fn build_assistant_catalog() -> Vec<AssistantRuntime> {
    let kinds = [
        RuntimeKind::ClaudeCode,
        RuntimeKind::GeminiCli,
        RuntimeKind::CodexCli,
    ];
    kinds
        .into_iter()
        .map(|kind| {
            let fallback = format!("{} adapter unavailable", kind.runtime_key());
            let (available, binary) =
                acp::probe_runtime(kind.runtime_key()).unwrap_or((false, fallback));
            let version = if available && binary != "npx" {
                detect_binary_version(&binary)
            } else {
                None
            };
            AssistantRuntime {
                key: kind.runtime_key().to_string(),
                label: kind.label().to_string(),
                binary,
                available,
                version,
            }
        })
        .collect()
}

pub(crate) fn normalize_runtime_key(runtime: &str) -> Option<&'static str> {
    RuntimeKind::from_str(runtime).map(|k| k.runtime_key())
}

#[tauri::command]
pub(crate) fn detect_assistants() -> Result<Vec<AssistantRuntime>, String> {
    Ok(assistant_catalog().clone())
}
