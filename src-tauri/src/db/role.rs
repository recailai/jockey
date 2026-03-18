use crate::db::{get_state, with_db};
use crate::types::*;
use crate::{acp, now_ms, resolve_chat_cwd};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

pub(crate) fn upsert_role(
    state: State<'_, AppState>,
    team_id: String,
    role_name: String,
    runtime_kind: String,
    system_prompt: String,
    model: Option<String>,
    mode: Option<String>,
    mcp_servers_json: Option<String>,
    config_options_json: Option<String>,
    auto_approve: Option<bool>,
) -> Result<Role, String> {
    let now = now_ms();
    let existing_id = with_db(get_state(&state), |conn| {
        conn.query_row(
            "SELECT id FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![&team_id, &role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;

    let role = Role {
        id: existing_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id,
        role_name,
        runtime_kind,
        system_prompt,
        model,
        mode,
        mcp_servers_json: mcp_servers_json.unwrap_or_else(|| "[]".to_string()),
        config_options_json: config_options_json.unwrap_or_else(|| "{}".to_string()),
        auto_approve: auto_approve.unwrap_or(true),
        created_at: now,
        updated_at: now,
    };

    with_db(get_state(&state), |conn| {
        conn.execute(
            "INSERT INTO roles (id, team_id, role_name, runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(team_id, role_name) DO UPDATE SET
               runtime_kind = excluded.runtime_kind,
               system_prompt = excluded.system_prompt,
               model = excluded.model,
               mode = excluded.mode,
               mcp_servers_json = excluded.mcp_servers_json,
               config_options_json = excluded.config_options_json,
               auto_approve = excluded.auto_approve,
               updated_at = excluded.updated_at",
            params![
                &role.id,
                &role.team_id,
                &role.role_name,
                &role.runtime_kind,
                &role.system_prompt,
                &role.model,
                &role.mode,
                &role.mcp_servers_json,
                &role.config_options_json,
                role.auto_approve,
                role.created_at,
                role.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;

    Ok(role)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleInput {
    pub(crate) team_id: String,
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) system_prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) mcp_servers_json: Option<String>,
    pub(crate) config_options_json: Option<String>,
    pub(crate) auto_approve: Option<bool>,
}

#[tauri::command]
pub(crate) async fn upsert_role_cmd(
    state: State<'_, AppState>,
    input: RoleInput,
) -> Result<Role, String> {
    let role = upsert_role(
        state.clone(),
        input.team_id,
        input.role_name,
        input.runtime_kind,
        input.system_prompt,
        input.model,
        input.mode,
        input.mcp_servers_json,
        input.config_options_json,
        input.auto_approve,
    )?;
    let runtime_for_warmup = role.runtime_kind.clone();
    let role_for_warmup = role.role_name.clone();
    let cwd_for_warmup = resolve_chat_cwd(get_state(&state), Some(&role.team_id));
    tauri::async_runtime::spawn(async move {
        acp::prewarm_role(&runtime_for_warmup, &role_for_warmup, &cwd_for_warmup, None).await;
    });
    Ok(role)
}

fn role_from_row(row: &rusqlite::Row) -> rusqlite::Result<Role> {
    Ok(Role {
        id: row.get(0)?,
        team_id: row.get(1)?,
        role_name: row.get(2)?,
        runtime_kind: row.get(3)?,
        system_prompt: row.get(4)?,
        model: row.get(5)?,
        mode: row.get(6)?,
        mcp_servers_json: row
            .get::<_, Option<String>>(7)?
            .unwrap_or_else(|| "[]".to_string()),
        config_options_json: row
            .get::<_, Option<String>>(8)?
            .unwrap_or_else(|| "{}".to_string()),
        auto_approve: row.get::<_, Option<bool>>(9)?.unwrap_or(true),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(crate) fn list_roles_for_team(state: &AppState, team_id: &str) -> Result<Vec<Role>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, team_id, role_name, runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve, created_at, updated_at
                 FROM roles WHERE team_id = ?1 ORDER BY role_name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![team_id], role_from_row)
            .map_err(|e| e.to_string())?;
        let mut roles = Vec::new();
        for row in rows {
            roles.push(row.map_err(|e| e.to_string())?);
        }
        Ok(roles)
    })
}

#[tauri::command]
pub(crate) fn list_roles(
    state: State<'_, AppState>,
    team_id: Option<String>,
) -> Result<Vec<Role>, String> {
    let active = match team_id {
        Some(id) if crate::db::team_exists(get_state(&state), &id)? => id,
        _ => crate::db::ensure_default_team_id(get_state(&state))?,
    };
    list_roles_for_team(get_state(&state), &active)
}

pub(crate) fn load_role(
    state: &AppState,
    team_id: &str,
    role_name: &str,
) -> Result<Option<Role>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, team_id, role_name, runtime_kind, system_prompt, model, mode, mcp_servers_json, config_options_json, auto_approve, created_at, updated_at FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            role_from_row,
        ).optional().map_err(|e| e.to_string())
    })
}

pub(crate) fn load_role_runtime_kind(
    state: &AppState,
    team_id: &str,
    role_name: &str,
) -> Result<String, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT runtime_kind FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|item| item.unwrap_or_else(|| "mock".to_string()))
    })
}

pub(crate) fn resolve_role_runtime(
    state: State<'_, AppState>,
    team_id: &str,
    role_name: &str,
) -> Result<String, String> {
    load_role_runtime_kind(get_state(&state), team_id, role_name)
}

pub(crate) fn resolve_role_prompt(
    state: State<'_, AppState>,
    team_id: &str,
    role_name: &str,
) -> Result<String, String> {
    resolve_role_prompt_raw(get_state(&state), team_id, role_name)
}

pub(crate) fn resolve_role_prompt_raw(
    state: &AppState,
    team_id: &str,
    role_name: &str,
) -> Result<String, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT system_prompt FROM roles WHERE team_id = ?1 AND role_name = ?2",
            params![team_id, role_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|item| item.unwrap_or_default())
    })
}
