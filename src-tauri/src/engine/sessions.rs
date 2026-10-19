//! Session-level rules the engine applies on every client's behalf: which engine a persona
//! runs on, what a turn's `@mentions` route to, and the title a fresh session earns from its
//! first message. Each of these used to live in the desktop UI (and was copied into the web
//! gateway), so other clients either re-implemented them or went without.

use crate::types::AppState;

pub(crate) const DEFAULT_ROLE: &str = "Developer";

fn runtime_key(raw: &str) -> String {
    crate::assistant::normalize_runtime_key(raw)
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

/// The project's pinned engine for the persona, then the persona's own default, then any
/// installed agent — each only if it is actually available on this machine. Returns
/// `(runtime key, profile id)`.
pub(crate) async fn preferred_runtime(
    state: &AppState,
    project_id: Option<&str>,
    role: &str,
) -> Option<(String, String)> {
    let catalog = tokio::task::spawn_blocking(crate::assistant::cached_assistant_catalog)
        .await
        .ok()?;
    let available: Vec<_> = catalog.into_iter().filter(|a| a.available).collect();
    let find = |want: &str| {
        let want = runtime_key(want);
        available.iter().find(|a| runtime_key(&a.key) == want)
    };
    let pin = crate::db::project_agent::load_project_agent_config(state, project_id, role)
        .ok()
        .and_then(|c| c.runtime_kind);
    let persona = crate::db::role::load_role_runtime_kind_scoped(state, role, project_id).ok();
    pin.as_deref()
        .and_then(find)
        .or_else(|| persona.as_deref().and_then(find))
        .or(available.first())
        .map(|a| (a.key.clone(), a.profile_id.clone()))
}

/// Whether the runtime takes extra input into a running turn (vs. only between turns).
pub(crate) async fn supports_steer(runtime: &str) -> bool {
    let want = runtime_key(runtime);
    tokio::task::spawn_blocking(crate::assistant::cached_assistant_catalog)
        .await
        .map(|catalog| {
            catalog.iter().any(|a| {
                runtime_key(&a.key) == want
                    && a.input_delivery.get("strategy").and_then(|s| s.as_str()) == Some("steer")
            })
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

pub(crate) struct Route {
    /// Roles the turn runs, in order. Empty only for `/app_*` commands.
    pub(crate) targets: Vec<String>,
    pub(crate) label: String,
    /// A single explicit mention makes that persona (and its engine) the session's active one,
    /// same as picking it in the composer.
    pub(crate) activate: Option<(String, Option<String>, Option<String>)>,
}

/// Resolve the `@mentions` in `text` against the session's personas. Unknown roles are an
/// error rather than silently becoming prompt text the agent never sees routed.
pub(crate) fn route(
    state: &AppState,
    project_id: Option<&str>,
    active_role: &str,
    text: &str,
) -> Result<Route, String> {
    if text.starts_with("/app_") {
        return Ok(Route {
            targets: Vec::new(),
            label: active_role.to_string(),
            activate: None,
        });
    }
    let mentioned = crate::parser::parse_route_input(text).role_names;
    if mentioned.is_empty() {
        return Ok(Route {
            targets: vec![active_role.to_string()],
            label: active_role.to_string(),
            activate: None,
        });
    }
    let roles = crate::db::role::list_roles_by_project(state, project_id)?;
    let mut targets = Vec::new();
    let mut resolved = Vec::new();
    for name in &mentioned {
        let lookup = if name.eq_ignore_ascii_case("assistant") {
            DEFAULT_ROLE
        } else {
            name.as_str()
        };
        // Project-scoped rows sort first, so the first case-insensitive hit is the right one.
        let role = roles
            .iter()
            .find(|r| r.role_name.eq_ignore_ascii_case(lookup));
        let is_runtime_name = crate::runtime_kind::RuntimeKind::from_str(lookup).is_some();
        match role {
            Some(role) => {
                if !targets.contains(&role.role_name) {
                    targets.push(role.role_name.clone());
                    resolved.push(role);
                }
            }
            None if is_runtime_name => targets.push(lookup.to_string()),
            None => return Err(format!("role not found: {name}")),
        }
    }
    let activate = match (targets.as_slice(), resolved.as_slice()) {
        ([only], [role]) if only != active_role => {
            let runtime = Some(role.runtime_kind.clone()).filter(|r| !r.trim().is_empty());
            let profile = runtime
                .as_ref()
                .map(|_| role.runtime_profile_id.clone())
                .filter(|p| !p.trim().is_empty());
            Some((role.role_name.clone(), runtime, profile))
        }
        _ => None,
    };
    Ok(Route {
        label: targets.join(" & "),
        targets,
        activate,
    })
}

// ---------------------------------------------------------------------------
// Auto-title
// ---------------------------------------------------------------------------

const TITLE_MAX_CHARS: usize = 40;
const TITLE_FALLBACK_CHARS: usize = 30;

/// Placeholder names a session gets before anyone names it: the desktop's `Session_1`
/// (and its `_copyN` variants) plus the `Session_1_N` the engine generates on collisions.
pub(crate) fn is_default_title(title: &str) -> bool {
    let Some(rest) = title.strip_prefix("Session_1") else {
        return false;
    };
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    rest.is_empty()
        || rest.strip_prefix("_copy").is_some_and(digits)
        || rest
            .strip_prefix('_')
            .is_some_and(|n| !n.is_empty() && digits(n))
}

/// A short title from the first message: mentions, skill refs and a leading slash command are
/// dropped, words are kept up to 40 characters, and spaces become underscores.
pub(crate) fn derive_title(text: &str, now_ms: i64) -> String {
    let mut words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| !w.starts_with('@') && !w.starts_with('#'))
        .collect();
    if words.first().is_some_and(|w| w.starts_with('/')) {
        words.remove(0);
    }
    let cleaned = words.join(" ");
    let mut title = String::new();
    for w in &words {
        let candidate = if title.is_empty() {
            w.to_string()
        } else {
            format!("{title} {w}")
        };
        if candidate.chars().count() > TITLE_MAX_CHARS {
            break;
        }
        title = candidate;
    }
    if title.is_empty() {
        title = cleaned.chars().take(TITLE_FALLBACK_CHARS).collect();
    }
    let title = title.split_whitespace().collect::<Vec<_>>().join("_");
    if title.is_empty() {
        format!("Session_{now_ms}")
    } else {
        title
    }
}

/// Name a still-unnamed session after its first user message. Returns the new title.
pub(crate) fn auto_title(
    state: &AppState,
    session_id: &str,
    current_title: &str,
    text: &str,
) -> Option<String> {
    if !is_default_title(current_title) {
        return None;
    }
    if crate::db::app_session::count_user_messages(state, session_id).ok()? > 0 {
        return None;
    }
    let desired = derive_title(text, crate::now_ms());
    crate::db::app_session::rename_app_session_unique(state, session_id, &desired).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_titles_cover_desktop_and_engine_placeholders() {
        for t in [
            "Session_1",
            "Session_1_copy",
            "Session_1_copy3",
            "Session_1_2",
        ] {
            assert!(is_default_title(t), "{t}");
        }
        for t in ["Session_12", "Session_1_x", "fix_login", "Session_1_"] {
            assert!(!is_default_title(t), "{t}");
        }
    }

    #[test]
    fn derive_title_drops_mentions_and_commands_and_caps_length() {
        assert_eq!(
            derive_title("@Developer fix the login page", 1),
            "fix_the_login_page"
        );
        assert_eq!(derive_title("/compact now please", 1), "now_please");
        assert_eq!(derive_title("#skill   tidy   imports", 1), "tidy_imports");
        let long = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let title = derive_title(long, 1);
        assert!(title.chars().count() <= TITLE_MAX_CHARS);
        assert!(title.starts_with("alpha_beta"));
        assert_eq!(derive_title("@Developer", 42), "Session_42");
        assert_eq!(
            derive_title(&"x".repeat(80), 1).chars().count(),
            TITLE_FALLBACK_CHARS
        );
    }
}
