use crate::db::{get_state, with_db};
use crate::error::AppError;
use crate::now_ms;
use crate::runtime_profile;
use crate::types::*;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

fn validate_role_name(role_name: &str) -> Result<(), String> {
    if role_name.is_empty() {
        return Err(AppError::validation("role name required").to_string());
    }
    if role_name.chars().any(|c| c.is_whitespace()) {
        return Err(AppError::validation("role name cannot contain spaces").to_string());
    }
    if !role_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            AppError::validation("role name only allows letters, numbers, - and _").to_string(),
        );
    }
    Ok(())
}

pub(crate) fn upsert_role_with_id(
    state: &AppState,
    id: Option<String>,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
    model: Option<String>,
    mode: Option<String>,
    mcp_servers_json: Option<String>,
    config_options_json: Option<String>,
    config_option_defs_json: Option<String>,
    auto_approve: Option<bool>,
    project_id: Option<String>,
) -> Result<Role, String> {
    let role_name = role_name.trim().to_string();
    validate_role_name(&role_name)?;
    let runtime_profile_id = runtime_profile::profile_id(&runtime_kind);
    let runtime_kind = runtime_profile::runtime_key(&runtime_profile_id);
    let now = now_ms();
    let normalized_pid = project_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let existing = with_db(state, |conn| {
        crate::db::project::ensure_project_exists(conn, normalized_pid.as_deref())?;
        if let Some(ref rid) = id.as_ref().filter(|s| !s.trim().is_empty()) {
            let hit = conn
                .query_row(
                    "SELECT id, role_name, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id
                     FROM roles WHERE id = ?1 LIMIT 1",
                    params![rid],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<bool>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| AppError::db(e.to_string()).to_string())?;
            if let Some((rid_val, _rname, mcp, cfg, cfg_defs, approve, pid)) = hit {
                return Ok(Some((rid_val, mcp, cfg, cfg_defs, approve, pid)));
            }
        }

        let name_hit: Option<(String, String, Option<String>, Option<String>, Option<String>, Option<bool>, Option<String>)> = conn
            .query_row(
                "SELECT id, role_name, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id
                 FROM roles WHERE lower(role_name) = lower(?1) AND ((project_id = ?2) OR (project_id IS NULL AND ?2 IS NULL) OR (project_id = '' AND ?2 IS NULL)) LIMIT 1",
                params![&role_name, &normalized_pid],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<bool>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        if let Some((found_id, _existing_name, mcp, cfg, cfg_defs, approve, pid)) = name_hit {
            return Ok(Some((found_id, mcp, cfg, cfg_defs, approve, pid)));
        }
        Ok(None)
    })?;

    let target_id = existing
        .as_ref()
        .map(|(rid, _, _, _, _, _)| rid.clone())
        .or_else(|| id.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let mcp = mcp_servers_json
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|(_, mcp, _, _, _, _)| mcp.clone())
        })
        .unwrap_or_else(|| "[]".to_string());
    let cfg = config_options_json
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|(_, _, cfg, _, _, _)| cfg.clone())
        })
        .unwrap_or_else(|| "{}".to_string());
    let cfg_defs = config_option_defs_json
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|(_, _, _, defs, _, _)| defs.clone())
        })
        .unwrap_or_else(|| "[]".to_string());
    let approve = auto_approve
        .or_else(|| existing.as_ref().and_then(|(_, _, _, _, v, _)| *v))
        .unwrap_or(true);
    let resolved_project_id = normalized_pid.or_else(|| {
        existing
            .as_ref()
            .and_then(|(_, _, _, _, _, pid)| pid.clone())
    });

    with_db(state, |conn| {
        let exists_by_id: i64 = conn
            .query_row(
                "SELECT count(1) FROM roles WHERE id = ?1",
                params![&target_id],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if exists_by_id > 0 {
            conn.execute(
                "UPDATE roles SET
                   role_name = ?2,
                   runtime_kind = ?3,
                   runtime_profile_id = ?4,
                   system_prompt = ?5,
                   model = ?6,
                   mode = ?7,
                   mcp_servers_json = ?8,
                   config_options_json = ?9,
                   config_option_defs_json = ?10,
                   auto_approve = ?11,
                   project_id = ?12,
                   updated_at = ?13
                 WHERE id = ?1",
                params![
                    &target_id,
                    &role_name,
                    &runtime_kind,
                    &runtime_profile_id,
                    &system_prompt,
                    &model,
                    &mode,
                    &mcp,
                    &cfg,
                    &cfg_defs,
                    approve,
                    &resolved_project_id,
                    now,
                ],
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        } else {
            conn.execute(
                "INSERT INTO roles (id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    &target_id,
                    &role_name,
                    &runtime_kind,
                    &runtime_profile_id,
                    &system_prompt,
                    &model,
                    &mode,
                    &mcp,
                    &cfg,
                    &cfg_defs,
                    approve,
                    &resolved_project_id,
                    now,
                    now,
                ],
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        }
        Ok(())
    })?;

    let runtime_launch_method = crate::acp::adapter_launch_method(&runtime_kind);
    let role = Role {
        id: target_id,
        role_name,
        runtime_kind,
        runtime_profile_id,
        runtime_launch_method,
        system_prompt,
        model,
        mode,
        mcp_servers_json: mcp,
        config_options_json: cfg,
        config_option_defs_json: cfg_defs,
        auto_approve: approve,
        project_id: resolved_project_id,
        created_at: now,
        updated_at: now,
    };
    state.role_cache.clear();
    Ok(role)
}

pub(crate) fn upsert_role(
    state: &AppState,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
    model: Option<String>,
    mode: Option<String>,
    mcp_servers_json: Option<String>,
    config_options_json: Option<String>,
    config_option_defs_json: Option<String>,
    auto_approve: Option<bool>,
    project_id: Option<String>,
) -> Result<Role, String> {
    upsert_role_with_id(
        state,
        None,
        role_name,
        runtime_kind,
        system_prompt,
        model,
        mode,
        mcp_servers_json,
        config_options_json,
        config_option_defs_json,
        auto_approve,
        project_id,
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleInput {
    pub(crate) id: Option<String>,
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) runtime_profile_id: Option<String>,
    pub(crate) system_prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) mcp_servers_json: Option<String>,
    pub(crate) config_options_json: Option<String>,
    pub(crate) config_option_defs_json: Option<String>,
    pub(crate) auto_approve: Option<bool>,
    pub(crate) project_id: Option<String>,
}

#[tauri::command]
pub(crate) async fn upsert_role_cmd(
    state: State<'_, AppState>,
    input: RoleInput,
) -> Result<Role, String> {
    let role = upsert_role_with_id(
        get_state(&state),
        input.id,
        input.role_name,
        input.runtime_profile_id.unwrap_or(input.runtime_kind),
        input.system_prompt,
        input.model,
        input.mode,
        input.mcp_servers_json,
        input.config_options_json,
        input.config_option_defs_json,
        input.auto_approve,
        input.project_id,
    )?;
    Ok(role)
}

fn role_from_row(row: &rusqlite::Row) -> rusqlite::Result<Role> {
    let runtime_kind = row.get::<_, String>(2)?;
    let runtime_profile_id = row
        .get::<_, Option<String>>(3)?
        .unwrap_or_else(|| runtime_profile::profile_id(&runtime_kind));
    let runtime_launch_method = crate::acp::adapter_launch_method(&runtime_kind);
    Ok(Role {
        id: row.get(0)?,
        role_name: row.get(1)?,
        runtime_kind,
        runtime_profile_id,
        runtime_launch_method,
        system_prompt: row.get(4)?,
        model: row.get(5)?,
        mode: row.get(6)?,
        mcp_servers_json: row
            .get::<_, Option<String>>(7)?
            .unwrap_or_else(|| "[]".to_string()),
        config_options_json: row
            .get::<_, Option<String>>(8)?
            .unwrap_or_else(|| "{}".to_string()),
        config_option_defs_json: row
            .get::<_, Option<String>>(9)?
            .unwrap_or_else(|| "[]".to_string()),
        auto_approve: row.get::<_, Option<bool>>(10)?.unwrap_or(true),
        project_id: row.get::<_, Option<String>>(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

pub(crate) fn list_roles_by_project(
    state: &AppState,
    project_id: Option<&str>,
) -> Result<Vec<Role>, String> {
    with_db(state, |conn| {
        let (sql, params_vec): (&str, Vec<rusqlite::types::Value>) = if let Some(pid) =
            project_id.filter(|s| !s.trim().is_empty())
        {
            (
                "SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at
                 FROM roles WHERE project_id = ?1 OR project_id IS NULL OR project_id = '' ORDER BY (project_id IS NOT NULL AND project_id != '') DESC, role_name ASC",
                vec![pid.to_string().into()],
            )
        } else {
            (
                "SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at
                 FROM roles ORDER BY role_name ASC",
                vec![],
            )
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_vec.iter()), role_from_row)
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let mut roles = Vec::new();
        for row in rows {
            roles.push(row.map_err(|e| AppError::db(e.to_string()).to_string())?);
        }
        Ok(roles)
    })
}

pub(crate) fn list_all_roles(state: &AppState) -> Result<Vec<Role>, String> {
    list_roles_by_project(state, None)
}

#[tauri::command]
pub(crate) fn list_roles(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<Role>, String> {
    list_roles_by_project(get_state(&state), project_id.as_deref())
}

pub(crate) fn delete_role_internal(
    state: &AppState,
    role_name: &str,
    project_id: Option<&str>,
    role_id: Option<&str>,
) -> Result<(), String> {
    let role_name = role_name.trim().to_string();
    if role_name.is_empty() && role_id.is_none() {
        return Err(AppError::validation("role name or id required").to_string());
    }
    let normalized_pid = project_id.filter(|s| !s.trim().is_empty());
    with_db(state, |conn| {
        let role_info: Option<(String, String, Option<String>)> = if let Some(rid) = role_id {
            conn.query_row(
                "SELECT id, role_name, project_id FROM roles WHERE id = ?1",
                params![rid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| AppError::db(e.to_string()).to_string())?
        } else {
            conn.query_row(
                "SELECT id, role_name, project_id FROM roles WHERE lower(role_name) = lower(?1) AND ((project_id = ?2) OR (project_id IS NULL AND ?2 IS NULL) OR (project_id = '' AND ?2 IS NULL)) LIMIT 1",
                params![&role_name, &normalized_pid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).optional().map_err(|e| AppError::db(e.to_string()).to_string())?
        };

        let (target_id, target_name, target_pid) = match role_info {
            Some(info) => info,
            None => {
                return Err(AppError::not_found(format!("role not found: {role_name}")).to_string())
            }
        };

        let has_global_fallback: bool = if target_pid.is_some() {
            conn.query_row(
                "SELECT COUNT(1) FROM roles WHERE lower(role_name) = lower(?1) AND (project_id IS NULL OR project_id = '')",
                params![&target_name],
                |r| r.get::<_, i64>(0),
            ).map(|c| c > 0).unwrap_or(false)
        } else {
            false
        };

        if !has_global_fallback {
            let active_using_count: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM app_sessions WHERE closed_at IS NULL AND active_role = ?1 AND ((project_id = ?2) OR (project_id IS NULL AND ?2 IS NULL) OR (project_id = '' AND ?2 IS NULL))",
                    params![&target_name, &target_pid],
                    |row| row.get(0),
                )
                .map_err(|e| AppError::db(e.to_string()).to_string())?;
            if active_using_count > 0 {
                return Err(AppError::invalid_input(format!(
                    "role is active in {active_using_count} open session(s); switch role first",
                ))
                .to_string());
            }
        }
        conn.execute("DELETE FROM roles WHERE id = ?1", params![&target_id])
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        let remaining_with_name: i64 = conn
            .query_row(
                "SELECT count(1) FROM roles WHERE lower(role_name) = lower(?1)",
                params![&target_name],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if remaining_with_name == 0 {
            conn.execute(
                "DELETE FROM app_session_roles WHERE role_name = ?1",
                params![&target_name],
            )
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        }
        Ok(())
    })?;
    state.role_cache.clear();
    Ok(())
}

#[tauri::command]
pub(crate) fn delete_role_cmd(
    state: State<'_, AppState>,
    role_name: String,
    project_id: Option<String>,
    role_id: Option<String>,
) -> Result<(), String> {
    delete_role_internal(
        get_state(&state),
        &role_name,
        project_id.as_deref(),
        role_id.as_deref(),
    )
}

pub(crate) fn load_role(state: &AppState, role_name: &str) -> Result<Option<Role>, String> {
    load_role_scoped(state, role_name, None)
}

pub(crate) fn load_role_scoped(
    state: &AppState,
    role_name: &str,
    project_id: Option<&str>,
) -> Result<Option<Role>, String> {
    let cache_key = match project_id {
        Some(pid) if !pid.trim().is_empty() => format!("{}:{}", pid, role_name),
        _ => format!("global:{}", role_name),
    };
    if let Some(cached) = state.role_cache.get(&cache_key) {
        return Ok(Some((**cached).clone()));
    }
    let result = with_db(state, |conn| {
        if let Some(pid) = project_id.filter(|s| !s.trim().is_empty()) {
            let hit = conn
                .query_row(
                    "SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at
                     FROM roles WHERE lower(role_name) = lower(?1) AND project_id = ?2 LIMIT 1",
                    params![role_name, pid],
                    role_from_row,
                )
                .optional()
                .map_err(|e| AppError::db(e.to_string()).to_string())?;
            if hit.is_some() {
                return Ok(hit);
            }
        }
        let global_hit = conn
            .query_row(
                "SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at
                 FROM roles WHERE lower(role_name) = lower(?1) AND (project_id IS NULL OR project_id = '') ORDER BY updated_at DESC LIMIT 1",
                params![role_name],
                role_from_row,
            )
            .optional()
            .map_err(|e| AppError::db(e.to_string()).to_string())?;
        if global_hit.is_some() {
            return Ok(global_hit);
        }

        conn.query_row(
            "SELECT id, role_name, runtime_kind, runtime_profile_id, system_prompt, model, mode, mcp_servers_json, config_options_json, config_option_defs_json, auto_approve, project_id, created_at, updated_at
             FROM roles WHERE lower(role_name) = lower(?1) ORDER BY updated_at DESC LIMIT 1",
            params![role_name],
            role_from_row,
        )
        .optional()
        .map_err(|e| AppError::db(e.to_string()).to_string())
    })?;
    if let Some(ref role) = result {
        // Only memoize an answer that actually belongs to the scope that was asked for. The
        // last-resort query above has no project filter, so a global lookup can land on a
        // project-scoped row; storing that under `global:{name}` served one project's engine
        // as every project's answer, permanently, and `resolve_runtime_for_role` reads exactly
        // this path.
        let requested_project = project_id.map(str::trim).filter(|p| !p.is_empty());
        let resolved_project = role
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let scope_matches = match requested_project {
            // A project lookup legitimately falls back to the global row, which is the correct
            // resolution for that project and safe to remember under its key.
            Some(pid) => resolved_project.is_none() || resolved_project == Some(pid),
            None => resolved_project.is_none(),
        };
        if scope_matches {
            state
                .role_cache
                .insert(cache_key, std::sync::Arc::new(role.clone()));
        }
    }
    Ok(result)
}

pub(crate) fn update_role_config_option_defs_if_changed(
    state: &AppState,
    role_name: &str,
    runtime_kind: Option<&str>,
    config_option_defs_json: &str,
) -> Result<bool, String> {
    with_db(state, |conn| {
        // These defs are the runtime's own option catalog, so they legitimately apply to every
        // same-named persona running that runtime — but the guard below must select by the same
        // predicate the UPDATE uses. Reading an arbitrary row (`LIMIT 1`, unscoped) meant a
        // persona row on another engine could veto the write, leaving every row's defs stale.
        let existing: Option<String> = conn
            .query_row(
                "SELECT config_option_defs_json FROM roles
                 WHERE role_name = ?1 AND (?2 IS NULL OR runtime_kind = ?2)
                 ORDER BY updated_at DESC LIMIT 1",
                params![role_name, runtime_kind],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::db(e.to_string()).to_string())?
            .flatten();

        let changed = match existing {
            Some(current) => {
                let current_v = serde_json::from_str::<serde_json::Value>(&current).ok();
                let next_v =
                    serde_json::from_str::<serde_json::Value>(config_option_defs_json).ok();
                match (current_v, next_v) {
                    (Some(a), Some(b)) => a != b,
                    _ => current != config_option_defs_json,
                }
            }
            None => false,
        };
        if !changed {
            return Ok(false);
        }

        conn.execute(
            "UPDATE roles
             SET config_option_defs_json = ?1, updated_at = ?2
             WHERE role_name = ?3 AND (?4 IS NULL OR runtime_kind = ?4)",
            params![config_option_defs_json, now_ms(), role_name, runtime_kind],
        )
        .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(true)
    })
    .map(|changed| {
        if changed {
            state.role_cache.clear();
        }
        changed
    })
}

pub(crate) fn load_role_runtime_kind(state: &AppState, role_name: &str) -> Result<String, String> {
    load_role_runtime_kind_scoped(state, role_name, None)
}

pub(crate) fn load_role_runtime_kind_scoped(
    state: &AppState,
    role_name: &str,
    project_id: Option<&str>,
) -> Result<String, String> {
    if let Some(role) = load_role_scoped(state, role_name, project_id)? {
        return Ok(role.runtime_kind);
    }
    if let Some(kind) = crate::runtime_kind::RuntimeKind::from_str(role_name) {
        return Ok(kind.runtime_key().to_string());
    }
    Ok("mock".to_string())
}

pub(crate) fn resolve_role_runtime(state: &AppState, role_name: &str) -> Result<String, String> {
    load_role_runtime_kind(state, role_name)
}

pub(crate) fn seed_default_roles(state: &AppState) -> Result<(), String> {
    let roles = list_all_roles(state)?;
    if roles
        .iter()
        .all(|r| !r.role_name.eq_ignore_ascii_case("developer"))
    {
        upsert_role(
            state,
            "Developer".to_string(),
            "claude-native".to_string(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            Some(true),
            None,
        )?;
    }
    Ok(())
}

pub(crate) fn reassign_role_project(
    state: &AppState,
    role_name: &str,
    project_id: Option<String>,
    role_id: Option<String>,
) -> Result<Role, String> {
    let now = crate::now_ms();
    let normalized_pid = project_id.filter(|s| !s.trim().is_empty());
    let updated_id = with_db(state, |conn| {
        crate::db::project::ensure_project_exists(conn, normalized_pid.as_deref())?;
        let rid = if let Some(id) = role_id.filter(|s| !s.trim().is_empty()) {
            id
        } else {
            conn.query_row(
                "SELECT id FROM roles WHERE lower(role_name) = lower(?1) LIMIT 1",
                params![role_name],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_e| AppError::not_found(format!("role not found: {role_name}")).to_string())?
        };

        conn.execute(
            "UPDATE roles SET project_id = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![&normalized_pid, now, &rid],
        )
        .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(rid)
    })?;
    state.role_cache.clear();
    let roles = list_all_roles(state)?;
    roles
        .into_iter()
        .find(|r| r.id == updated_id)
        .ok_or_else(|| format!("failed to reload role: {role_name}"))
}

#[tauri::command]
pub(crate) fn reassign_role_project_cmd(
    state: State<'_, AppState>,
    role_name: String,
    project_id: Option<String>,
    role_id: Option<String>,
) -> Result<Role, String> {
    reassign_role_project(get_state(&state), &role_name, project_id, role_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::DbPool;
    use dashmap::DashMap;
    use std::sync::Arc;

    /// Creates only the `roles` table these tests exercise, rather than the whole app schema,
    /// so role-resolution coverage cannot be broken by unrelated schema changes elsewhere.
    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(
            dir.path().join("test.sqlite3"),
            2,
            "PRAGMA foreign_keys = ON;",
        )
        .expect("pool");
        {
            let conn = pool.get().expect("conn");
            conn.execute_batch(
                "CREATE TABLE roles (
                   id TEXT PRIMARY KEY,
                   role_name TEXT NOT NULL,
                   runtime_kind TEXT NOT NULL,
                   runtime_profile_id TEXT,
                   system_prompt TEXT NOT NULL,
                   model TEXT,
                   mode TEXT,
                   mcp_servers_json TEXT DEFAULT '[]',
                   config_options_json TEXT DEFAULT '{}',
                   config_option_defs_json TEXT DEFAULT '[]',
                   auto_approve INTEGER DEFAULT 1,
                   project_id TEXT,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL
                 );",
            )
            .expect("roles table");
        }
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    fn seed(state: &AppState, name: &str, runtime: &str, project: Option<&str>) {
        upsert_role(
            state,
            name.to_string(),
            runtime.to_string(),
            String::new(),
            None,
            None,
            None,
            None,
            None,
            Some(true),
            project.map(str::to_string),
        )
        .expect("seed role");
    }

    #[test]
    fn a_project_role_is_never_cached_as_the_global_one() {
        // The last-resort lookup has no project filter, so asking for the global row when only
        // a project-scoped one exists returns that project's row. Remembering it under
        // `global:{name}` is what made `resolve_runtime_for_role` report one project's engine
        // as everyone's, for the rest of the process.
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        seed(&state, "Architect", "codex-cli", Some("p1"));

        let fallback = load_role_scoped(&state, "Architect", None)
            .expect("load")
            .expect("falls back to the only row that exists");
        assert_eq!(fallback.runtime_kind, "codex-cli");
        assert!(
            !state.role_cache.contains_key("global:Architect"),
            "a project row must not be memoized as the global answer"
        );

        // A later global row must win immediately rather than being shadowed by the cache.
        seed(&state, "Architect", "claude-native", None);
        let global = load_role_scoped(&state, "Architect", None)
            .expect("load")
            .expect("global row");
        assert_eq!(global.runtime_kind, "claude-native");
        assert!(global.project_id.is_none());
    }

    #[test]
    fn project_and_global_roles_sharing_a_name_do_not_shadow_each_other() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        seed(&state, "Developer", "claude-native", None);
        seed(&state, "Developer", "codex-cli", Some("p1"));

        // Resolve in both directions, twice, so a poisoned cache entry would surface.
        for _ in 0..2 {
            let scoped = load_role_scoped(&state, "Developer", Some("p1"))
                .expect("load")
                .expect("project row");
            assert_eq!(scoped.runtime_kind, "codex-cli");

            let global = load_role_scoped(&state, "Developer", None)
                .expect("load")
                .expect("global row");
            assert_eq!(global.runtime_kind, "claude-native");
        }
    }

    #[test]
    fn a_project_without_its_own_role_resolves_to_the_global_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        seed(&state, "Reviewer", "claude-native", None);

        let resolved = load_role_scoped(&state, "Reviewer", Some("p-unknown"))
            .expect("load")
            .expect("global fallback");
        assert_eq!(resolved.runtime_kind, "claude-native");
        // Caching that resolution per project is correct — it is the right answer for p-unknown.
        assert!(state.role_cache.contains_key("p-unknown:Reviewer"));
    }
}
