//! Per-(project, persona) agent configuration: which engine a persona runs on in this
//! project, and that engine's knobs (model / effort / fast / 1M context / mode).
//!
//! Keyed by persona rather than by runtime. Two personas on the same CLI are separate
//! working setups — "Architect on codex with high effort" and "Reviewer on codex with low
//! effort" must not overwrite each other, which is exactly what a runtime-keyed table did.
//!
//! The persona's own `runtime_kind` / `model` act as the seed for a project that has not
//! tuned it yet; once tuned, this row wins.
//!
//! `project_id` is empty for sessions that belong to no project, so the memory still works
//! outside a project.

use crate::db::with_db;
use crate::now_ms;
use crate::types::AppState;
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

fn normalize_project_id(project_id: Option<&str>) -> String {
    project_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("")
        .to_string()
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectAgentConfig {
    /// `None` means the project has not pinned an engine for this persona; callers fall back
    /// to the persona's own `runtime_kind`.
    pub(crate) runtime_kind: Option<String>,
    pub(crate) options: Vec<(String, String)>,
}

/// The engine this persona is pinned to in this project, if any.
pub(crate) fn load_project_agent_pin(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
) -> Result<Option<String>, String> {
    let pid = normalize_project_id(project_id);
    let pin: Option<Option<String>> = with_db(state, |conn| {
        conn.query_row(
            "SELECT runtime_kind FROM project_agent_configs
             WHERE project_id = ?1 AND role_name = ?2",
            params![pid, role_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    Ok(pin.flatten().filter(|r| !r.trim().is_empty()))
}

/// Every persona this project has pinned an engine for, as `role_name -> runtime_kind`.
pub(crate) fn list_project_agent_pins(
    state: &AppState,
    project_id: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let pid = normalize_project_id(project_id);
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT role_name, runtime_kind FROM project_agent_configs
                 WHERE project_id = ?1 AND runtime_kind IS NOT NULL AND trim(runtime_kind) <> ''",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![pid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(Result::ok).collect())
    })
}

/// Knobs this persona uses on one specific engine in this project. Scoped by engine because a
/// model id or effort level belongs to the CLI that offers it — switching engines used to
/// clear them, which lost the user's Claude setup the moment they tried Codex.
pub(crate) fn load_project_agent_options(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
    runtime_kind: &str,
) -> Result<Vec<(String, String)>, String> {
    let pid = normalize_project_id(project_id);
    let raw: Option<String> = with_db(state, |conn| {
        conn.query_row(
            "SELECT config_options_json FROM project_agent_runtime_configs
             WHERE project_id = ?1 AND role_name = ?2 AND runtime_kind = ?3",
            params![pid, role_name, runtime_kind],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    Ok(raw.map(|r| parse_config_map(&r)).unwrap_or_default())
}

/// Pin plus the knobs belonging to the pinned engine — what the picker renders.
pub(crate) fn load_project_agent_config(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
) -> Result<ProjectAgentConfig, String> {
    let runtime_kind = load_project_agent_pin(state, project_id, role_name)?;
    let options = match runtime_kind.as_deref() {
        Some(rt) => load_project_agent_options(state, project_id, role_name, rt)?,
        None => Vec::new(),
    };
    Ok(ProjectAgentConfig {
        runtime_kind,
        options,
    })
}

fn write(
    state: &AppState,
    pid: &str,
    role_name: &str,
    runtime_kind: Option<&str>,
    options_json: &str,
) -> Result<(), String> {
    let now = now_ms();
    with_db(state, |conn| {
        crate::db::project::ensure_project_exists(conn, Some(pid))?;
        conn.execute(
            "INSERT INTO project_agent_configs (project_id, role_name, runtime_kind, config_options_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id, role_name) DO UPDATE SET
               runtime_kind = COALESCE(excluded.runtime_kind, project_agent_configs.runtime_kind),
               config_options_json = excluded.config_options_json,
               updated_at = excluded.updated_at",
            params![pid, role_name, runtime_kind, options_json, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Set one knob for this persona on one engine. `runtime_kind` defaults to the pinned engine.
pub(crate) fn save_project_agent_config_option(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
    runtime_kind: Option<&str>,
    config_id: &str,
    value: &str,
) -> Result<ProjectAgentConfig, String> {
    let key = config_id.trim();
    if key.is_empty() {
        return Err("config option id required".to_string());
    }
    let pin = load_project_agent_pin(state, project_id, role_name)?;
    let explicit = runtime_kind
        .map(str::to_string)
        .filter(|r| !r.trim().is_empty());
    let Some(runtime) = explicit.clone().or(pin.clone()) else {
        return Err("no engine selected for this persona".to_string());
    };

    let pid = normalize_project_id(project_id);
    // Configuring an engine implies running on it. Without this, a knob set before the
    // project ever pinned an engine would be written to a row nothing reads back.
    if pin.is_none() {
        write(state, &pid, role_name, Some(&runtime), "{}")?;
    }
    let mut map: Map<String, Value> =
        load_project_agent_options(state, project_id, role_name, &runtime)?
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
    // An empty value clears the override rather than pinning an empty string, so the
    // persona default becomes visible again.
    if value.trim().is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), Value::String(value.trim().to_string()));
    }
    write_runtime_options(
        state,
        &pid,
        role_name,
        &runtime,
        &Value::Object(map).to_string(),
    )?;
    load_project_agent_config(state, project_id, role_name)
}

/// Pin the engine this persona runs on in this project. The knobs of both the old and the new
/// engine are kept — each engine owns its own row — so switching away and back restores the
/// setup you had, instead of resetting every field to blank.
pub(crate) fn save_project_agent_runtime(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
    runtime_kind: &str,
) -> Result<ProjectAgentConfig, String> {
    let pid = normalize_project_id(project_id);
    write(state, &pid, role_name, Some(runtime_kind), "{}")?;
    load_project_agent_config(state, project_id, role_name)
}

/// Register that this persona is now in use under this project, seeded with the runtime it
/// actually resolved to on first send. A no-op if the project has already tuned this persona
/// (via `save_project_agent_runtime`/`save_project_agent_config_option`) — that explicit pick
/// must always win over this lazy background registration, so this only ever inserts, never
/// updates.
pub(crate) fn ensure_project_agent_registered(
    state: &AppState,
    project_id: Option<&str>,
    role_name: &str,
    runtime_kind: &str,
) -> Result<(), String> {
    let pid = normalize_project_id(project_id);
    let now = now_ms();
    with_db(state, |conn| {
        crate::db::project::ensure_project_exists(conn, Some(&pid))?;
        conn.execute(
            "INSERT INTO project_agent_configs (project_id, role_name, runtime_kind, config_options_json, updated_at)
             VALUES (?1, ?2, ?3, '{}', ?4)
             ON CONFLICT(project_id, role_name) DO NOTHING",
            params![pid, role_name, runtime_kind, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn write_runtime_options(
    state: &AppState,
    pid: &str,
    role_name: &str,
    runtime_kind: &str,
    options_json: &str,
) -> Result<(), String> {
    let now = now_ms();
    with_db(state, |conn| {
        crate::db::project::ensure_project_exists(conn, Some(pid))?;
        conn.execute(
            "INSERT INTO project_agent_runtime_configs (project_id, role_name, runtime_kind, config_options_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id, role_name, runtime_kind) DO UPDATE SET
               config_options_json = excluded.config_options_json,
               updated_at = excluded.updated_at",
            params![pid, role_name, runtime_kind, options_json, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(crate) fn parse_config_map(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v.as_object().map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .filter(|(_, v)| !v.trim().is_empty())
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::db::pool::DbPool;
    use dashmap::DashMap;
    use std::sync::Arc;

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(
            dir.path().join("test.sqlite3"),
            2,
            "PRAGMA foreign_keys = ON;",
        )
        .expect("pool");
        {
            let conn = pool.get().expect("conn");
            init_db(&conn).expect("init_db");
            for pid in ["p", "a", "b"] {
                conn.execute(
                    "INSERT OR IGNORE INTO projects (id, name, root_path, created_at, updated_at) \
                     VALUES (?1, ?1, ?2, 1, 1)",
                    rusqlite::params![pid, format!("/tmp/{pid}")],
                )
                .expect("insert test project");
            }
        }
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    fn opts(state: &AppState, project: Option<&str>, role: &str) -> Vec<(String, String)> {
        load_project_agent_config(state, project, role)
            .expect("load")
            .options
    }

    #[test]
    fn parse_config_map_drops_blank_and_non_string_values() {
        let parsed = parse_config_map(r#"{"model":"claude-opus-5","effort":"","fast":true}"#);
        assert_eq!(
            parsed,
            vec![("model".to_string(), "claude-opus-5".to_string())]
        );
    }

    #[test]
    fn two_personas_on_the_same_engine_do_not_share_knobs() {
        // The whole reason for re-keying by persona: a runtime-keyed table made Architect
        // and Reviewer overwrite each other whenever both ran on the same CLI.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, Some("p"), "Architect", "codex-cli").expect("pin");
        save_project_agent_runtime(&state, Some("p"), "Reviewer", "codex-cli").expect("pin");
        save_project_agent_config_option(&state, Some("p"), "Architect", None, "effort", "xhigh")
            .expect("set");
        save_project_agent_config_option(&state, Some("p"), "Reviewer", None, "effort", "low")
            .expect("set");

        assert_eq!(
            opts(&state, Some("p"), "Architect"),
            vec![("effort".to_string(), "xhigh".to_string())]
        );
        assert_eq!(
            opts(&state, Some("p"), "Reviewer"),
            vec![("effort".to_string(), "low".to_string())]
        );
    }

    #[test]
    fn the_same_persona_is_tuned_separately_per_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_config_option(
            &state,
            Some("a"),
            "Dev",
            Some("claude-native"),
            "model",
            "claude-opus-5",
        )
        .expect("set");
        save_project_agent_config_option(
            &state,
            Some("b"),
            "Dev",
            Some("claude-native"),
            "model",
            "claude-sonnet-5",
        )
        .expect("set");

        assert_eq!(
            opts(&state, Some("a"), "Dev"),
            vec![("model".to_string(), "claude-opus-5".to_string())]
        );
        assert_eq!(
            opts(&state, Some("b"), "Dev"),
            vec![("model".to_string(), "claude-sonnet-5".to_string())]
        );
    }

    #[test]
    fn each_engine_keeps_its_own_knobs_across_a_switch() {
        // `claude-opus-5` means nothing to Antigravity, so the knobs must not carry over —
        // but they also must not be destroyed: switching back has to restore the Claude setup
        // instead of handing the user a blank form.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, None, "Dev", "claude-native").expect("pin");
        save_project_agent_config_option(&state, None, "Dev", None, "model", "claude-opus-5")
            .expect("set");

        let switched =
            save_project_agent_runtime(&state, None, "Dev", "antigravity-cli").expect("switch");
        assert_eq!(switched.runtime_kind.as_deref(), Some("antigravity-cli"));
        assert!(
            switched.options.is_empty(),
            "the new engine starts from its own (empty) setup"
        );

        let back = save_project_agent_runtime(&state, None, "Dev", "claude-native").expect("back");
        assert_eq!(
            back.options,
            vec![("model".to_string(), "claude-opus-5".to_string())],
            "switching back must restore the engine's remembered knobs"
        );
    }

    #[test]
    fn re_pinning_the_same_engine_keeps_the_knobs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, None, "Dev", "codex-cli").expect("pin");
        save_project_agent_config_option(&state, None, "Dev", None, "effort", "high").expect("set");
        let again = save_project_agent_runtime(&state, None, "Dev", "codex-cli").expect("re-pin");
        assert_eq!(
            again.options,
            vec![("effort".to_string(), "high".to_string())]
        );
    }

    #[test]
    fn blank_value_clears_the_override_instead_of_pinning_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, None, "Dev", "claude-native").expect("pin");
        save_project_agent_config_option(&state, None, "Dev", None, "effort", "xhigh")
            .expect("set");
        save_project_agent_config_option(&state, None, "Dev", None, "model", "claude-opus-5")
            .expect("set");
        let cleared = save_project_agent_config_option(&state, None, "Dev", None, "effort", "")
            .expect("clear");
        assert_eq!(
            cleared.options,
            vec![("model".to_string(), "claude-opus-5".to_string())]
        );
    }

    #[test]
    fn false_is_kept_as_an_explicit_toggle_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, None, "Dev", "claude-native").expect("pin");
        let saved = save_project_agent_config_option(&state, None, "Dev", None, "fast", "false")
            .expect("set false");

        assert_eq!(
            saved.options,
            vec![("fast".to_string(), "false".to_string())]
        );
    }

    #[test]
    fn an_untuned_persona_reports_no_engine_so_callers_fall_back_to_its_seed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let config = load_project_agent_config(&state, Some("p"), "Never-Touched").expect("load");
        assert!(config.runtime_kind.is_none());
        assert!(config.options.is_empty());
    }

    #[test]
    fn first_send_registers_the_persona_with_the_resolved_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        ensure_project_agent_registered(&state, Some("p"), "Dev", "codex-cli").expect("register");
        let config = load_project_agent_config(&state, Some("p"), "Dev").expect("load");
        assert_eq!(config.runtime_kind.as_deref(), Some("codex-cli"));
        assert!(config.options.is_empty());
    }

    #[test]
    fn registration_never_clobbers_a_pick_the_picker_already_made() {
        // The picker's explicit choice must always win over this lazy, best-effort
        // registration that fires on every send — otherwise a slow/late send could
        // silently undo a model the user just picked.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);

        save_project_agent_runtime(&state, Some("p"), "Dev", "claude-native").expect("pin");
        save_project_agent_config_option(&state, Some("p"), "Dev", None, "model", "claude-opus-5")
            .expect("set");

        ensure_project_agent_registered(&state, Some("p"), "Dev", "codex-cli").expect("register");

        let config = load_project_agent_config(&state, Some("p"), "Dev").expect("load");
        assert_eq!(config.runtime_kind.as_deref(), Some("claude-native"));
        assert_eq!(
            config.options,
            vec![("model".to_string(), "claude-opus-5".to_string())]
        );
    }
}
