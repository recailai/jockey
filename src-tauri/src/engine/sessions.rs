//! Session-level rules the engine applies on every client's behalf: which engine a persona
//! runs on, what a turn's `@mentions` route to, and the title a fresh session earns from its
//! first message. Each of these used to live in the desktop UI (and was copied into the web
//! gateway), so other clients either re-implemented them or went without.

use super::events::{self, ProjectChange, SessionChange};
use crate::types::{AppSession, AppSessionUpdate, AppState, Project, SessionSummary};

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

/// The engine a persona runs on in a session — the one rule every caller uses (the turn,
/// cancel / steer / mode / config, persona activation), so a mode change can never target a
/// different process than the turn ran on.
///
/// - The session's **active** persona runs on the session's selected engine (`requested`, the
///   composer's current pick, else the session row).
/// - Any **other** persona resolves its own: project pin, then the session-role binding, then
///   the persona's default, then the role name itself when it names a runtime. The session's
///   engine is only a last resort — passing it to every persona ran e.g. `@agy` on Claude.
///
/// `project_id_override` serves draft sessions, which have no row to read the project from.
pub(crate) fn resolve_role_runtime(
    state: &AppState,
    app_session_id: &str,
    role_name: &str,
    requested: Option<&str>,
    project_id_override: Option<&str>,
) -> Option<String> {
    let head = crate::db::app_session::load_session_head(state, app_session_id)
        .ok()
        .flatten();
    let project_id = project_id_override
        .map(str::to_string)
        .or_else(|| head.as_ref().and_then(|h| h.project_id.clone()));
    let non_empty = |r: &str| !r.trim().is_empty();
    let session_runtime = head
        .as_ref()
        .and_then(|h| h.runtime_kind.clone())
        .filter(|r| non_empty(r));
    let is_active = head
        .as_ref()
        .is_some_and(|h| h.active_role.eq_ignore_ascii_case(role_name));
    if is_active {
        if let Some(rt) = requested
            .filter(|r| non_empty(r))
            .map(str::to_string)
            .or_else(|| session_runtime.clone())
        {
            return Some(runtime_key(&rt));
        }
    }
    let pin = crate::db::project_agent::load_project_agent_config(
        state,
        project_id.as_deref(),
        role_name,
    )
    .ok()
    .and_then(|c| c.runtime_kind);
    let bound = crate::db::app_session_role::load_app_session_bound_runtime(
        state,
        app_session_id,
        role_name,
    )
    .ok()
    .flatten();
    let persona =
        crate::db::role::load_role_runtime_kind_scoped(state, role_name, project_id.as_deref())
            .ok();
    let by_name =
        crate::runtime_kind::RuntimeKind::from_str(role_name).map(|_| role_name.to_string());
    pin.or(bound)
        .or(persona)
        .or(by_name)
        .or(session_runtime)
        .filter(|r| non_empty(r))
        .map(|r| runtime_key(&r))
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
// Session and project operations — the only writers; each one announces itself
// ---------------------------------------------------------------------------

pub(crate) fn summaries(
    state: &AppState,
    project_id: Option<&str>,
    closed: bool,
) -> Result<Vec<SessionSummary>, String> {
    let mut rows = crate::db::app_session::list_session_summaries(state, project_id, closed, None)?;
    for row in &mut rows {
        row.running = super::turns::is_busy(&row.id);
    }
    Ok(rows)
}

pub(crate) struct NewSession<'a> {
    pub(crate) title: Option<&'a str>,
    pub(crate) project_id: Option<&'a str>,
    pub(crate) runtime_kind: Option<&'a str>,
    pub(crate) runtime_profile_id: Option<&'a str>,
    /// Client-chosen id, so a draft session keeps its identity once it becomes real.
    pub(crate) id: Option<&'a str>,
}

/// Create a session. One that names no engine gets the preferred one for its project, so
/// every client sees the same engine instead of each picking its own default.
pub(crate) async fn create_session(
    state: &AppState,
    new: NewSession<'_>,
) -> Result<AppSession, String> {
    let project_id = new.project_id.map(str::trim).filter(|p| !p.is_empty());
    let preferred = match new.runtime_kind.filter(|r| !r.trim().is_empty()) {
        Some(_) => None,
        None => preferred_runtime(state, project_id, DEFAULT_ROLE).await,
    };
    let (runtime, profile) = match &preferred {
        Some((key, profile)) => (Some(key.as_str()), Some(profile.as_str())),
        None => (new.runtime_kind, new.runtime_profile_id),
    };
    let session = crate::db::app_session::create_app_session_internal(
        state,
        new.title.filter(|t| !t.trim().is_empty()),
        project_id,
        runtime,
        profile,
        new.id,
    )?;
    events::session_changed(state, &session.id, SessionChange::Created);
    Ok(session)
}

pub(crate) fn update_session(
    state: &AppState,
    id: &str,
    update: AppSessionUpdate,
) -> Result<(), String> {
    crate::db::app_session::update_app_session_internal(state, id, update)?;
    events::session_changed(state, id, SessionChange::Updated);
    Ok(())
}

/// Close a session. Returns `false` if it was already closed.
pub(crate) fn close_session(state: &AppState, id: &str) -> Result<bool, String> {
    // Stop the running turn through the engine so it still records what it produced and
    // announces turn/finished, instead of being killed by the provider-slot cleanup below.
    if super::turns::is_busy(id) {
        super::service::cancel_in_background(id);
    }
    let closed = crate::db::app_session::close_app_session_internal(state, id)?;
    if closed {
        events::session_changed(state, id, SessionChange::Closed);
    }
    Ok(closed)
}

pub(crate) fn reopen_session(state: &AppState, id: &str) -> Result<AppSession, String> {
    let session = crate::db::app_session::reopen_app_session_internal(state, id)?;
    events::session_changed(state, id, SessionChange::Reopened);
    // Input queued before the session was closed resumes now.
    if let Some(app) = events::app() {
        super::service::kick(&app, id);
    }
    Ok(session)
}

/// Import CLI transcripts into a project. Scanning reads provider files, so run it on the
/// blocking pool.
pub(crate) async fn import_sessions(
    state: &AppState,
    project_id: &str,
    external_session_ids: Option<Vec<String>>,
) -> Result<Vec<AppSession>, String> {
    let worker_state = state.clone_refs();
    let pid = project_id.to_string();
    let imported = tokio::task::spawn_blocking(move || {
        crate::importer::import_project_sessions_internal(
            &worker_state,
            &pid,
            external_session_ids.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    for session in &imported {
        events::session_changed(state, &session.id, SessionChange::Imported);
    }
    Ok(imported)
}

pub(crate) fn create_project(
    state: &AppState,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let project = crate::db::project::create_project_internal(state, name, root_path)?;
    events::project_changed(&project.id, ProjectChange::Created);
    Ok(project)
}

pub(crate) fn delete_project(state: &AppState, id: &str) -> Result<(), String> {
    crate::db::project::delete_project_internal(state, id)?;
    // Sessions of a deleted project would otherwise stay open and runnable, orphaned from
    // every project list.
    for session in crate::db::app_session::list_session_summaries(state, Some(id), false, None)? {
        let _ = close_session(state, &session.id);
    }
    events::project_changed(id, ProjectChange::Deleted);
    Ok(())
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
    session_id: &str,
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
            // Same precedence the turn itself uses for a non-active persona: the project's
            // pin, then the persona's default — normalised, so legacy spellings such as
            // `native:agy` never reach the session row or the views.
            // Not yet active, so this resolves the persona's own engine (pin, binding,
            // default) — normalised, so legacy spellings like `native:agy` never leak out.
            let runtime =
                resolve_role_runtime(state, session_id, &role.role_name, None, project_id);
            let profile = runtime.as_deref().map(crate::runtime_profile::profile_id);
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

#[cfg(test)]
mod resolver_tests {
    use super::*;
    use crate::db::{init_db, DbPool};
    use dashmap::DashMap;
    use std::sync::Arc;

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(dir.path().join("t.sqlite3"), 2, "PRAGMA foreign_keys = ON;")
            .expect("pool");
        init_db(&pool.get().expect("conn")).expect("init");
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    fn role(state: &AppState, name: &str, runtime: &str) {
        crate::db::role::upsert_role(
            state,
            name.into(),
            runtime.into(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("role");
    }

    #[test]
    fn active_persona_uses_the_selection_others_resolve_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        role(&state, "Developer", "claude-native");
        role(&state, "agy", "native:agy");
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("s"),
            None,
            Some("codex-cli"),
            None,
            None,
        )
        .unwrap();
        // Active persona: the composer's pick wins, then the session's engine.
        assert_eq!(
            resolve_role_runtime(&state, &session.id, "Developer", Some("pi-cli"), None),
            Some(runtime_key("pi-cli"))
        );
        assert_eq!(
            resolve_role_runtime(&state, &session.id, "developer", None, None),
            Some(runtime_key("codex-cli")),
            "persona names compare case-insensitively"
        );
        // Another persona ignores the selection and normalises its legacy spelling.
        assert_eq!(
            resolve_role_runtime(&state, &session.id, "agy", Some("codex-cli"), None),
            Some(runtime_key("native:agy"))
        );
        assert_ne!(runtime_key("native:agy"), "native:agy");
    }

    #[test]
    fn project_pin_beats_the_persona_default_for_other_personas() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        role(&state, "Developer", "claude-native");
        role(&state, "agy", "native:agy");
        let project = crate::db::project::create_project_internal(
            &state,
            "p".into(),
            dir.path().to_string_lossy().to_string(),
        )
        .unwrap();
        crate::db::project_agent::save_project_agent_runtime(
            &state,
            Some(&project.id),
            "agy",
            "codex-cli",
        )
        .unwrap();
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("s"),
            Some(&project.id),
            Some("claude-native"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            resolve_role_runtime(&state, &session.id, "agy", None, None),
            Some(runtime_key("codex-cli"))
        );
    }

    fn role_with(state: &AppState, name: &str, runtime: &str, project: Option<&str>) {
        crate::db::role::upsert_role(
            state,
            name.into(),
            runtime.into(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            project.map(str::to_string),
        )
        .expect("role");
    }

    fn project(state: &AppState, dir: &tempfile::TempDir, name: &str) -> String {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        crate::db::project::create_project_internal(
            state,
            name.into(),
            root.to_string_lossy().to_string(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn deleting_a_global_role_is_refused_while_project_sessions_use_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        role_with(&state, "Reviewer", "claude-native", None);
        let pid = project(&state, &dir, "p");
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("s"),
            Some(&pid),
            None,
            None,
            None,
        )
        .unwrap();
        crate::db::app_session::update_app_session_internal(
            &state,
            &session.id,
            crate::types::AppSessionUpdate {
                title: None,
                active_role: Some("Reviewer".into()),
                runtime_kind: None,
                runtime_profile_id: None,
                cwd: None,
            },
        )
        .unwrap();
        let err =
            crate::db::role::delete_role_internal(&state, "Reviewer", None, None).unwrap_err();
        assert!(err.contains("active in 1 open session"), "{err}");
    }

    #[test]
    fn saving_a_global_role_for_a_project_creates_a_project_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        role_with(&state, "Writer", "claude-native", None);
        let global = crate::db::role::load_role_scoped(&state, "Writer", None)
            .unwrap()
            .unwrap();
        let pid = project(&state, &dir, "p");
        crate::db::role::upsert_role_with_id(
            &state,
            Some(global.id.clone()),
            "Writer".into(),
            "codex-cli".into(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(pid.clone()),
        )
        .unwrap();
        state.role_cache.clear();
        let still_global = crate::db::role::load_role_scoped(&state, "Writer", None)
            .unwrap()
            .unwrap();
        assert_eq!(still_global.id, global.id);
        assert!(
            still_global.project_id.is_none(),
            "the global row stays global"
        );
        let scoped = crate::db::role::load_role_scoped(&state, "Writer", Some(&pid))
            .unwrap()
            .unwrap();
        assert_ne!(scoped.id, global.id);
        assert_eq!(scoped.project_id.as_deref(), Some(pid.as_str()));
    }
}
