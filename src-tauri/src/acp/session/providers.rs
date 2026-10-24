use crate::acp::protocol as acp;
use serde_json::json;

use super::super::adapter::acp_log;
use super::super::transport::AcpConnection;

/// Sends the runtime's saved LLM providers to a freshly initialized adapter. Provider settings
/// belong to the adapter process, so this runs on every cold start, before any session call.
/// A failure is logged and kept for the settings page, never raised: the session still works on
/// the adapter's own routing.
pub(super) async fn apply_saved_providers(
    conn: &AcpConnection,
    runtime_key: &str,
    init: &acp::InitializeResponse,
) {
    if init.agent_capabilities.providers.is_none() {
        return;
    }
    let pending = crate::llm_providers::pending_requests(runtime_key);
    if pending.is_empty() {
        return;
    }
    let mut failures = Vec::new();
    for (provider_id, req) in &pending {
        let outcome = match req {
            Ok(req) => conn
                .request::<_, acp::SetProviderResponse>("providers/set", req)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.clone()),
        };
        if let Err(e) = outcome {
            failures.push(format!("{provider_id}: {e}"));
        }
    }
    acp_log(
        "providers.apply",
        json!({ "runtime": runtime_key, "count": pending.len(), "failures": failures }),
    );
    crate::llm_providers::record_apply(runtime_key, &failures);
}
