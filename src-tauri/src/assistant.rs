use crate::acp;
use crate::acp::session::adapter_runtime::{AnyRuntimeAdapter, RuntimeAdapter};
use crate::runtime_kind::RuntimeKind;
use crate::runtime_profile::{all_profiles, builtin_profiles, RuntimeCapabilities, RuntimeFamily};
use crate::types::*;
use serde_json::to_value;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const ASSISTANT_CATALOG_TTL: Duration = Duration::from_secs(15);

type CatalogCache = Option<(Instant, Vec<AssistantRuntime>)>;

fn catalog_cache() -> &'static Mutex<CatalogCache> {
    static CACHE: OnceLock<Mutex<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

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

pub(crate) fn build_assistant_catalog() -> Vec<AssistantRuntime> {
    let kinds = [
        RuntimeKind::ClaudeNative,
        RuntimeKind::ClaudeCode,
        RuntimeKind::AntigravityCli,
        RuntimeKind::CodexCli,
        RuntimeKind::PiCli,
    ];
    let mut rows = kinds
        .into_iter()
        .map(|kind| {
            let profile = builtin_profiles()
                .into_iter()
                .find(|profile| profile.runtime_key == kind.runtime_key())
                .expect("builtin runtime profile must exist");
            let fallback = format!("{} adapter unavailable", kind.runtime_key());
            let (available, binary) =
                acp::probe_runtime(kind.runtime_key()).unwrap_or((false, fallback));
            let version_binary = binary
                .split_once(" (")
                .map(|(path, _)| path)
                .unwrap_or(&binary);
            let version = if available && version_binary != "npx" {
                detect_binary_version(version_binary)
            } else {
                None
            };
            let transport = acp::adapter_transport(kind.runtime_key())
                .unwrap_or_else(|| "unavailable".to_string());
            let launch_method = if available {
                acp::adapter_launch_method(kind.runtime_key())
            } else {
                None
            };
            let install_hint = if available {
                None
            } else {
                let h = kind.install_hint();
                if h.is_empty() {
                    None
                } else {
                    Some(h.to_string())
                }
            };
            let capabilities = if available {
                profile.capabilities
            } else {
                RuntimeCapabilities::unavailable()
            };
            let input_delivery = AnyRuntimeAdapter::resolve(kind.runtime_key())
                .map(|adapter| adapter.descriptor().input_delivery)
                .unwrap_or_else(|| {
                    AnyRuntimeAdapter::resolve("mock")
                        .expect("mock adapter")
                        .descriptor()
                        .input_delivery
                });
            let unavailable_reason = (!available).then_some(binary.clone());
            AssistantRuntime {
                key: kind.runtime_key().to_string(),
                profile_id: profile.id,
                label: profile.label,
                family: match profile.family {
                    RuntimeFamily::Native => "native".to_string(),
                    RuntimeFamily::Acp => "acp".to_string(),
                },
                binary,
                available,
                version,
                install_hint,
                unavailable_reason,
                launch_method,
                transport,
                capabilities: to_value(capabilities).unwrap_or_else(|_| serde_json::json!({})),
                input_delivery: to_value(input_delivery).unwrap_or_else(|_| serde_json::json!({})),
            }
        })
        .collect::<Vec<_>>();

    for profile in all_profiles()
        .into_iter()
        .filter(|profile| !profile.builtin)
    {
        let fallback = format!("{} adapter unavailable", profile.id);
        let (available, binary) = acp::probe_runtime(&profile.id).unwrap_or((false, fallback));
        let version_binary = binary
            .split_once(" (")
            .map(|(path, _)| path)
            .unwrap_or(&binary);
        let version = if available {
            detect_binary_version(version_binary)
        } else {
            None
        };
        let transport =
            acp::adapter_transport(&profile.id).unwrap_or_else(|| "unavailable".to_string());
        let launch_method = if available {
            acp::adapter_launch_method(&profile.id)
        } else {
            None
        };
        let unavailable_reason = (!available).then_some(binary.clone());
        let input_delivery = AnyRuntimeAdapter::resolve(&profile.runtime_key)
            .map(|adapter| adapter.descriptor().input_delivery)
            .unwrap_or_else(|| {
                AnyRuntimeAdapter::resolve("mock")
                    .expect("mock adapter")
                    .descriptor()
                    .input_delivery
            });
        rows.push(AssistantRuntime {
            key: profile.id.clone(),
            profile_id: profile.id,
            label: profile.label,
            family: "acp".to_string(),
            binary,
            available,
            version,
            install_hint: None,
            unavailable_reason,
            launch_method,
            transport,
            capabilities: to_value(if available {
                profile.capabilities
            } else {
                RuntimeCapabilities::unavailable()
            })
            .unwrap_or_else(|_| serde_json::json!({})),
            input_delivery: to_value(input_delivery).unwrap_or_else(|_| serde_json::json!({})),
        });
    }
    rows
}

pub(crate) fn cached_assistant_catalog() -> Vec<AssistantRuntime> {
    if let Ok(guard) = catalog_cache().lock() {
        if let Some((at, rows)) = guard.as_ref() {
            if at.elapsed() <= ASSISTANT_CATALOG_TTL {
                return rows.clone();
            }
        }
    }
    refresh_assistant_catalog()
}

pub(crate) fn refresh_assistant_catalog() -> Vec<AssistantRuntime> {
    let rows = build_assistant_catalog();
    if let Ok(mut guard) = catalog_cache().lock() {
        *guard = Some((Instant::now(), rows.clone()));
    }
    rows
}

pub(crate) fn normalize_runtime_key(runtime: &str) -> Option<&'static str> {
    crate::runtime_profile::runtime_key_static(runtime)
}

#[tauri::command]
pub(crate) async fn detect_assistants() -> Result<Vec<AssistantRuntime>, String> {
    tokio::task::spawn_blocking(|| {
        acp::clear_adapter_cache();
        acp::clear_discovered_catalogs();
        refresh_assistant_catalog()
    })
    .await
    .map_err(|e| e.to_string())
}
