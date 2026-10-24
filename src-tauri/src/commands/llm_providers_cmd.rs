//! Tauri commands for the per-runtime LLM provider settings. Storage, caching and validation
//! live in `crate::llm_providers`; these only shape payloads for the frontend.

use serde::Serialize;
use tauri::State;

use crate::acp::protocol as acp;
use crate::acp::session::probe::probe_runtime;
use crate::db::get_state;
use crate::llm_providers::{self, protocol_name, runtime_kind, ProviderConfigInput};
use crate::types::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveProvider {
    provider_id: String,
    supported: Vec<String>,
    required: bool,
    current_api_type: Option<String>,
    current_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedProviderConfig {
    provider_id: String,
    api_type: String,
    base_url: String,
    /// Names only; values never leave the keychain once saved.
    header_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedProviders {
    configs: Vec<SavedProviderConfig>,
    apply_error: Option<String>,
}

#[tauri::command]
pub(crate) async fn list_runtime_providers_cmd(
    runtime_key: String,
) -> Result<Option<Vec<LiveProvider>>, String> {
    let kind = runtime_kind(&runtime_key)?;
    tokio::task::spawn_blocking(move || {
        probe_runtime(kind, "providers/list", |conn, init| async move {
            if init.agent_capabilities.providers.is_none() {
                return Ok(None);
            }
            let resp: acp::ListProvidersResponse = conn
                .request("providers/list", acp::ListProvidersRequest::new())
                .await
                .map_err(|e| format!("providers/list: {e}"))?;
            Ok(Some(
                resp.providers
                    .into_iter()
                    .map(|p| LiveProvider {
                        provider_id: p.provider_id.to_string(),
                        supported: p.supported.iter().map(protocol_name).collect(),
                        required: p.required,
                        current_api_type: p.current.as_ref().map(|c| protocol_name(&c.api_type)),
                        current_base_url: p.current.map(|c| c.base_url),
                    })
                    .collect(),
            ))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn get_llm_provider_configs_cmd(runtime_key: String) -> SavedProviders {
    let configs = llm_providers::configs_for(&runtime_key)
        .into_iter()
        .map(|c| SavedProviderConfig {
            provider_id: c.provider_id,
            api_type: c.api_type,
            base_url: c.base_url,
            header_names: c.header_names,
        })
        .collect();
    SavedProviders {
        configs,
        apply_error: llm_providers::apply_error(&runtime_key),
    }
}

#[tauri::command]
pub(crate) async fn set_llm_provider_config_cmd(
    state: State<'_, AppState>,
    runtime_key: String,
    config: ProviderConfigInput,
) -> Result<(), String> {
    let state = get_state(&state).clone_refs();
    tokio::task::spawn_blocking(move || llm_providers::save(&state, &runtime_key, &config))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn clear_llm_provider_config_cmd(
    state: State<'_, AppState>,
    runtime_key: String,
    provider_id: String,
) -> Result<(), String> {
    let state = get_state(&state).clone_refs();
    tokio::task::spawn_blocking(move || llm_providers::clear(&state, &runtime_key, &provider_id))
        .await
        .map_err(|e| e.to_string())?
}
