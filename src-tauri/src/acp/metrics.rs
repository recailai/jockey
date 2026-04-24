use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRuntimeMetrics {
    pub runtime_key: String,
    pub spawn_count: u64,
    pub init_count: u64,
    pub session_start_count: u64,
    pub prompt_count: u64,
    pub error_count: u64,
    pub idle_reclaim_count: u64,
    pub total_spawn_ms: u64,
    pub total_init_ms: u64,
    pub total_session_start_ms: u64,
    pub total_prompt_ms: u64,
}

static METRICS: OnceLock<Mutex<HashMap<String, AcpRuntimeMetrics>>> = OnceLock::new();

fn metrics() -> &'static Mutex<HashMap<String, AcpRuntimeMetrics>> {
    METRICS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_runtime(runtime_key: &str, f: impl FnOnce(&mut AcpRuntimeMetrics)) {
    if let Ok(mut map) = metrics().lock() {
        let entry = map
            .entry(runtime_key.to_string())
            .or_insert_with(|| AcpRuntimeMetrics {
                runtime_key: runtime_key.to_string(),
                ..Default::default()
            });
        f(entry);
    }
}

pub(crate) fn record_spawn_latency(runtime_key: &str, ms: u128) {
    with_runtime(runtime_key, |m| {
        m.spawn_count += 1;
        m.total_spawn_ms = m.total_spawn_ms.saturating_add(ms as u64);
    });
}

pub(crate) fn record_init_latency(runtime_key: &str, ms: u128) {
    with_runtime(runtime_key, |m| {
        m.init_count += 1;
        m.total_init_ms = m.total_init_ms.saturating_add(ms as u64);
    });
}

pub(crate) fn record_session_start_latency(runtime_key: &str, ms: u128) {
    with_runtime(runtime_key, |m| {
        m.session_start_count += 1;
        m.total_session_start_ms = m.total_session_start_ms.saturating_add(ms as u64);
    });
}

pub(crate) fn record_prompt_latency(runtime_key: &str, ms: u128) {
    with_runtime(runtime_key, |m| {
        m.prompt_count += 1;
        m.total_prompt_ms = m.total_prompt_ms.saturating_add(ms as u64);
    });
}

pub(crate) fn record_error(runtime_key: &str) {
    with_runtime(runtime_key, |m| {
        m.error_count += 1;
    });
}

pub(crate) fn record_idle_reclaim(runtime_key: &str) {
    with_runtime(runtime_key, |m| {
        m.idle_reclaim_count += 1;
    });
}

pub fn snapshot() -> Vec<AcpRuntimeMetrics> {
    let mut items = metrics()
        .lock()
        .map(|map| map.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    items.sort_by(|a, b| a.runtime_key.cmp(&b.runtime_key));
    items
}
