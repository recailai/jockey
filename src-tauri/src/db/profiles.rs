use crate::db::with_db;
use crate::now_ms;
use crate::runtime_profile::{
    all_profiles, register_custom_profile, remove_custom_profile, RuntimeCapabilities,
    RuntimeFamily, RuntimeLaunchSpec, RuntimeProfile,
};
use crate::types::AppState;
use rusqlite::{params, Connection};
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeProfileInput {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) env_refs: Vec<String>,
    pub(crate) cwd_strategy: Option<String>,
}

fn validate_env_ref(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn profile_from_input(input: RuntimeProfileInput) -> Result<RuntimeProfile, String> {
    let id = input
        .id
        .unwrap_or_else(|| format!("acp:custom:{}", uuid::Uuid::new_v4()));
    let id = id.trim().to_ascii_lowercase();
    if !id.starts_with("acp:custom:") {
        return Err("custom ACP profile id must start with acp:custom:".to_string());
    }
    let suffix = id.trim_start_matches("acp:custom:");
    if suffix.is_empty()
        || !suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
    {
        return Err("custom ACP profile id suffix contains invalid characters".to_string());
    }
    let label = input.label.trim().to_string();
    let command = input.command.trim().to_string();
    if label.is_empty() || command.is_empty() {
        return Err("custom ACP label and command are required".to_string());
    }
    if input.env_refs.iter().any(|key| !validate_env_ref(key)) {
        return Err("envRefs must contain shell environment variable names only".to_string());
    }
    Ok(RuntimeProfile {
        id: id.clone(),
        label,
        family: RuntimeFamily::Acp,
        transport: "acp".to_string(),
        runtime_key: id,
        capabilities: RuntimeCapabilities::unavailable(),
        launch_spec: Some(RuntimeLaunchSpec {
            command,
            args: input.args,
            env_refs: input.env_refs,
            cwd_strategy: input.cwd_strategy.unwrap_or_else(|| "session".to_string()),
        }),
        version_requirement: Some("stable-schema-v1".to_string()),
        update_strategy: "user-managed".to_string(),
        builtin: false,
    })
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeProfile> {
    let id: String = row.get(0)?;
    let capabilities = row
        .get::<_, String>(6)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(RuntimeCapabilities::unavailable);
    let args = row
        .get::<_, String>(3)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let env_refs = row
        .get::<_, String>(4)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    Ok(RuntimeProfile {
        id: id.clone(),
        label: row.get(1)?,
        family: RuntimeFamily::Acp,
        transport: "acp".to_string(),
        runtime_key: id,
        capabilities,
        launch_spec: Some(RuntimeLaunchSpec {
            command: row.get(2)?,
            args,
            env_refs,
            cwd_strategy: row.get(5)?,
        }),
        version_requirement: Some("stable-schema-v1".to_string()),
        update_strategy: "user-managed".to_string(),
        builtin: false,
    })
}

pub(crate) fn load_custom_runtime_profiles(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, command, args_json, env_refs_json, cwd_strategy, capabilities_json
             FROM runtime_profiles ORDER BY label ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_profile)
        .map_err(|e| e.to_string())?;
    for row in rows {
        register_custom_profile(row.map_err(|e| e.to_string())?)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn list_runtime_profiles_cmd(
    state: State<'_, AppState>,
) -> Result<Vec<RuntimeProfile>, String> {
    with_db(state.inner(), load_custom_runtime_profiles)?;
    Ok(all_profiles())
}

#[tauri::command]
pub(crate) fn upsert_runtime_profile_cmd(
    state: State<'_, AppState>,
    input: RuntimeProfileInput,
) -> Result<RuntimeProfile, String> {
    let profile = profile_from_input(input)?;
    let launch = profile
        .launch_spec
        .as_ref()
        .ok_or_else(|| "custom ACP launch spec required".to_string())?;
    let args_json = serde_json::to_string(&launch.args).map_err(|e| e.to_string())?;
    let env_json = serde_json::to_string(&launch.env_refs).map_err(|e| e.to_string())?;
    let capabilities_json =
        serde_json::to_string(&profile.capabilities).map_err(|e| e.to_string())?;
    let now = now_ms();
    with_db(state.inner(), |conn| {
        conn.execute(
            "INSERT INTO runtime_profiles (id, label, command, args_json, env_refs_json, cwd_strategy, capabilities_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
               label = excluded.label,
               command = excluded.command,
               args_json = excluded.args_json,
               env_refs_json = excluded.env_refs_json,
               cwd_strategy = excluded.cwd_strategy,
               capabilities_json = excluded.capabilities_json,
               updated_at = excluded.updated_at",
            params![
                &profile.id,
                &profile.label,
                &launch.command,
                args_json,
                env_json,
                &launch.cwd_strategy,
                capabilities_json,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    register_custom_profile(profile.clone())?;
    Ok(profile)
}

#[tauri::command]
pub(crate) fn delete_runtime_profile_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    if !id.starts_with("acp:custom:") {
        return Err("only custom ACP profiles can be deleted".to_string());
    }
    with_db(state.inner(), |conn| {
        conn.execute("DELETE FROM runtime_profiles WHERE id = ?1", params![&id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    remove_custom_profile(&id);
    Ok(())
}
