use dashmap::DashMap;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Bump whenever the option JSON the catalog emits changes shape, so an entry built by an
/// older build is re-derived instead of being handed to a picker that expects new fields.
const CATALOG_SHAPE: u32 = 1;

/// How long a discovered catalog is trusted before the next refresh re-reads it from the
/// binary. Bounded staleness is what lets a CLI upgrade show up without restarting Jockey —
/// previously the only invalidation was an explicit `clear_runtime` at a call site, so a
/// forgotten call meant an entry lived until the process died.
const CATALOG_TTL: Duration = Duration::from_secs(10 * 60);

/// The option set for one runtime, together with what it was derived from. Readers compare
/// against this instead of trusting that a writer remembered to invalidate.
#[derive(Clone)]
struct CatalogOptions {
    values: Vec<Value>,
    shape: u32,
    at: Instant,
}

/// Everything discovered about one runtime. `options` is deliberately separate from `models`:
/// a chat turn learns the model ids on its own without ever building an option set, and
/// treating "models are known" as "the catalog is known" is what served the picker an empty
/// option set — no effort levels, no capability toggles.
#[derive(Clone, Default)]
struct RuntimeCatalog {
    models: Vec<String>,
    modes: Vec<String>,
    options: Option<CatalogOptions>,
}

static RUNTIME_CATALOGS: OnceLock<DashMap<String, RuntimeCatalog>> = OnceLock::new();
static RUNTIME_AVAILABLE_COMMANDS: OnceLock<DashMap<String, Vec<Value>>> = OnceLock::new();

fn runtime_catalogs() -> &'static DashMap<String, RuntimeCatalog> {
    RUNTIME_CATALOGS.get_or_init(DashMap::new)
}

fn runtime_available_commands() -> &'static DashMap<String, Vec<Value>> {
    RUNTIME_AVAILABLE_COMMANDS.get_or_init(DashMap::new)
}

fn with_catalog<T>(runtime_key: &str, f: impl FnOnce(&mut RuntimeCatalog) -> T) -> T {
    let mut entry = runtime_catalogs()
        .entry(runtime_key.to_string())
        .or_default();
    f(entry.value_mut())
}

pub(super) fn clear_all() {
    clear_discovered_catalogs();
    runtime_available_commands().clear();
}

pub fn clear_discovered_catalogs() {
    runtime_catalogs().clear();
}

pub(super) fn clear_runtime(runtime_key: &str) {
    runtime_catalogs().remove(runtime_key);
}

pub(super) fn clear_session(app_session_id: &str, runtime_key: &str, role_name: &str) {
    runtime_available_commands().remove(&session_role_key(app_session_id, runtime_key, role_name));
}

pub(super) fn remember_runtime_models(runtime_key: &str, mut models: Vec<String>) {
    models.sort_unstable();
    models.dedup();
    with_catalog(runtime_key, |catalog| catalog.models = models);
}

pub(super) fn has_discovered_models(runtime_key: &str) -> bool {
    runtime_catalogs()
        .get(runtime_key)
        .map(|c| !c.models.is_empty())
        .unwrap_or(false)
}

pub(super) fn remember_runtime_modes(runtime_key: &str, mut modes: Vec<String>) {
    modes.sort_unstable();
    modes.dedup();
    with_catalog(runtime_key, |catalog| catalog.modes = modes);
}

pub fn list_discovered_models(runtime_key: &str) -> Vec<String> {
    runtime_catalogs()
        .get(runtime_key)
        .map(|c| c.models.clone())
        .unwrap_or_default()
}

pub fn list_discovered_modes(runtime_key: &str) -> Vec<String> {
    runtime_catalogs()
        .get(runtime_key)
        .map(|c| c.modes.clone())
        .unwrap_or_default()
}

// Config option *definitions* (the schema of what options exist) are scoped per-runtime,
// not per-session-role. Two concurrent sessions on the same runtime share the same
// discovered option definitions (last-write-wins). Config option *values* are stored
// separately per-session-role in app_session_roles.config_options_json.
pub(super) fn remember_runtime_config_options(runtime_key: &str, options: Vec<Value>) {
    with_catalog(runtime_key, |catalog| {
        catalog.options = Some(CatalogOptions {
            values: options,
            shape: CATALOG_SHAPE,
            at: Instant::now(),
        });
    });
}

/// The cached option set, or `None` when there is none, when it was built by a build that
/// emitted a different shape, or when it has aged out. Refreshers ask this rather than asking
/// whether models happen to be known, so a partially-populated entry can never masquerade as
/// a complete catalog.
pub(super) fn fresh_config_options(runtime_key: &str) -> Option<Vec<Value>> {
    runtime_catalogs().get(runtime_key).and_then(|catalog| {
        catalog
            .options
            .as_ref()
            .filter(|opts| opts.shape == CATALOG_SHAPE && opts.at.elapsed() < CATALOG_TTL)
            .map(|opts| opts.values.clone())
    })
}

/// Whatever is cached, regardless of age — for callers rendering what is already known rather
/// than deciding whether to re-discover.
pub fn list_discovered_config_options(runtime_key: &str) -> Vec<Value> {
    runtime_catalogs()
        .get(runtime_key)
        .and_then(|c| c.options.as_ref().map(|o| o.values.clone()))
        .unwrap_or_default()
}

pub(super) fn remember_runtime_available_commands(
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
    commands: Vec<Value>,
) {
    if commands.is_empty() {
        return;
    }
    runtime_available_commands().insert(
        session_role_key(app_session_id, runtime_key, role_name),
        commands.clone(),
    );
    runtime_available_commands().insert(format!("{runtime_key}:{role_name}"), commands.clone());
    runtime_available_commands().insert(runtime_key.to_string(), commands.clone());
    if runtime_key.contains("claude") {
        runtime_available_commands().insert("claude-code".to_string(), commands.clone());
        runtime_available_commands().insert("claude-native".to_string(), commands);
    }
}

pub fn list_available_commands(
    app_session_id: &str,
    runtime_key: &str,
    role_name: &str,
) -> Vec<Value> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let cached = runtime_available_commands()
        .get(&session_role_key(app_session_id, runtime_key, role_name))
        .or_else(|| runtime_available_commands().get(&format!("{runtime_key}:{role_name}")))
        .or_else(|| runtime_available_commands().get(runtime_key))
        .or_else(|| {
            if runtime_key.contains("claude") {
                runtime_available_commands()
                    .get("claude-code")
                    .or_else(|| runtime_available_commands().get("claude-native"))
            } else {
                None
            }
        })
        .map(|v| v.clone())
        .unwrap_or_default();

    for item in cached {
        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
            if seen.insert(name.to_string()) {
                results.push(item);
            }
        }
    }

    results
}

fn session_role_key(app_session_id: &str, runtime_key: &str, role_name: &str) -> String {
    format!("{app_session_id}:{runtime_key}:{role_name}")
}

#[cfg(test)]
mod catalog_reuse_tests {
    use super::*;
    use serde_json::json;

    /// A chat turn remembers model ids without ever building a catalog, so "models are known"
    /// is not evidence that the option set is known. Both catalog refreshers used to reuse on
    /// that basis and returned an empty option set — the picker then showed a model list with
    /// no effort levels and no capability toggles.
    #[test]
    fn models_alone_are_never_mistaken_for_a_complete_catalog() {
        let runtime = "reuse-probe-runtime";
        clear_runtime(runtime);
        remember_runtime_models(runtime, vec!["m-1".to_string()]);

        assert!(has_discovered_models(runtime));
        assert!(
            fresh_config_options(runtime).is_none(),
            "a refresher asking this must be told to go discover, not handed an empty set"
        );

        remember_runtime_config_options(runtime, vec![json!({ "id": "model" })]);
        assert!(fresh_config_options(runtime).is_some());
        assert_eq!(
            list_discovered_models(runtime),
            vec!["m-1".to_string()],
            "writing options must not drop the models already learned"
        );
        clear_runtime(runtime);
    }

    /// Invalidation used to depend on some call site remembering to call `clear_runtime`. The
    /// entry now carries the shape it was built with, so an option set from a build that
    /// emitted a different shape is re-derived rather than served to a picker expecting new
    /// fields — nobody has to remember anything.
    #[test]
    fn an_entry_from_another_build_shape_is_not_reused() {
        let runtime = "shape-probe-runtime";
        clear_runtime(runtime);
        remember_runtime_config_options(runtime, vec![json!({ "id": "model" })]);
        assert!(fresh_config_options(runtime).is_some());

        with_catalog(runtime, |catalog| {
            if let Some(options) = catalog.options.as_mut() {
                options.shape = CATALOG_SHAPE + 1;
            }
        });
        assert!(
            fresh_config_options(runtime).is_none(),
            "a differently-shaped entry must be re-derived"
        );
        assert!(
            !list_discovered_config_options(runtime).is_empty(),
            "callers merely rendering what is known still see it; only reuse is refused"
        );
        clear_runtime(runtime);
    }

    /// Bounded staleness is what lets a CLI upgrade appear without restarting Jockey.
    #[test]
    fn an_aged_out_entry_is_not_reused() {
        let runtime = "ttl-probe-runtime";
        clear_runtime(runtime);
        remember_runtime_config_options(runtime, vec![json!({ "id": "model" })]);

        with_catalog(runtime, |catalog| {
            if let Some(options) = catalog.options.as_mut() {
                options.at = Instant::now() - CATALOG_TTL - Duration::from_secs(1);
            }
        });
        assert!(fresh_config_options(runtime).is_none());
        clear_runtime(runtime);
    }

    #[test]
    fn available_commands_are_cached_and_aliased() {
        let commands = vec![
            json!({ "name": "model", "description": "Configure model" }),
            json!({ "name": "commit", "description": "Commit changes" }),
        ];
        remember_runtime_available_commands(
            "test-session",
            "claude-code",
            "Developer",
            commands.clone(),
        );

        // Retrieved by session + role
        let found = list_available_commands("test-session", "claude-code", "Developer");
        assert_eq!(found.len(), 2);

        // Retrieved by aliased runtime claude-native
        let aliased = list_available_commands("different-session", "claude-native", "Architect");
        assert_eq!(aliased.len(), 2);
        assert_eq!(aliased[0]["name"], "model");
    }
}
