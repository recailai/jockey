//! Agent accounts: named credential profiles per CLI, bound to roles.
//!
//! A profile is a config dir (where the CLI keeps its login) plus extra env (API keys). A role
//! bound to a profile launches its CLI with that profile's env; an unbound role keeps the
//! CLI's own default login, which is never touched. Turns and prewarm both resolve their env
//! through `launch_env_for`, and every process slot compares env on reuse, so rebinding a role
//! takes effect on its next turn.

pub(crate) mod provider;
pub(crate) mod secrets;
pub(crate) mod store;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{get_state, with_db};
use crate::types::AppState;
use provider::{AccountStatus, CredentialProvider, ALL_PROVIDERS};
use store::AgentCredential;

const MANAGED_DIR: &str = "credentials";

/// Env a role's bound profile adds to its CLI launch; empty when the role uses the default
/// login. A bound profile whose secret cannot be read is an error rather than a silent fall
/// back to the default login, which would bill the wrong account.
pub(crate) fn launch_env_for(
    state: &AppState,
    role_name: &str,
    project_id: Option<&str>,
    runtime_kind: &str,
) -> Result<Vec<(String, String)>, String> {
    let Some(provider) = CredentialProvider::for_runtime(runtime_kind) else {
        return Ok(Vec::new());
    };
    let Some(role) = crate::db::role::load_role_scoped(state, role_name, project_id)? else {
        return Ok(Vec::new());
    };
    let credential = with_db(state, |conn| {
        match store::bound_credential_id(conn, &role.id, provider)? {
            Some(id) => store::get(conn, &id),
            None => Ok(None),
        }
    })?;
    match credential {
        Some(credential) => credential_env(&credential),
        None => Ok(Vec::new()),
    }
}

/// The bound profile's config-dir variable alone, for commands shown to or run by the user:
/// it keeps a resumed conversation on the same account without writing secrets anywhere.
pub(crate) fn config_dir_env_for(
    state: &AppState,
    role_name: &str,
    project_id: Option<&str>,
    runtime_kind: &str,
) -> Result<Option<(String, String)>, String> {
    let Some(var) = CredentialProvider::for_runtime(runtime_kind).and_then(|p| p.config_dir_env())
    else {
        return Ok(None);
    };
    Ok(launch_env_for(state, role_name, project_id, runtime_kind)?
        .into_iter()
        .find(|(key, _)| key == var))
}

fn credential_env(credential: &AgentCredential) -> Result<Vec<(String, String)>, String> {
    let provider = credential.provider;
    let mut env = Vec::new();
    if let (Some(var), Some(dir)) = (provider.config_dir_env(), credential.config_dir.as_deref()) {
        if credential.managed {
            // Picks up entries the default dir gained since the profile was created.
            provider.link_shared_entries(Path::new(dir));
            env.extend(provider.managed_env());
        }
        env.push((var.to_string(), dir.to_string()));
    }
    let secrets = secrets::load(&credential.id)
        .map_err(|e| format!("{} account \"{}\": {e}", provider.label(), credential.label))?;
    env.extend(secrets);
    Ok(env)
}

fn managed_root() -> Result<PathBuf, String> {
    crate::acp::app_data_dir()
        .map(|dir| dir.join(MANAGED_DIR))
        .ok_or_else(|| "app data dir is not initialised".to_string())
}

fn is_managed_path(dir: &Path) -> bool {
    managed_root().is_ok_and(|root| dir.starts_with(root))
}

fn validate_env_key(provider: CredentialProvider, key: &str) -> Result<(), String> {
    let valid = !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!("invalid env var name: {key}"));
    }
    if provider.config_dir_env() == Some(key) {
        return Err(format!("{key} is set from the profile's config dir"));
    }
    Ok(())
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn login_command(credential: &AgentCredential) -> Option<String> {
    let provider = credential.provider;
    let command = provider.login_command()?;
    let var = provider.config_dir_env()?;
    let dir = credential.config_dir.as_deref()?;
    Some(format!("{var}={} {command}", shell_quote(dir)))
}

// ── Views ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderView {
    key: &'static str,
    label: &'static str,
    /// The CLI's login can be relocated into a separate config dir.
    supports_sign_in: bool,
    login_hint: &'static str,
    suggested_env_keys: &'static [&'static str],
    default_config_dir: Option<String>,
    default_account: AccountStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialView {
    id: String,
    provider: CredentialProvider,
    label: String,
    config_dir: Option<String>,
    managed: bool,
    env_keys: Vec<String>,
    account: AccountStatus,
    login_command: Option<String>,
    used_by: Vec<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialsOverview {
    providers: Vec<ProviderView>,
    credentials: Vec<CredentialView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleCredentialBinding {
    provider: String,
    credential_id: String,
}

fn credential_view(credential: AgentCredential, used_by: Vec<String>) -> CredentialView {
    let account = match credential.config_dir.as_deref() {
        Some(dir) => credential.provider.account_status(Path::new(dir)),
        None => AccountStatus {
            signed_in: !credential.env_keys.is_empty(),
            identity: (!credential.env_keys.is_empty()).then(|| credential.env_keys.join(", ")),
        },
    };
    CredentialView {
        login_command: login_command(&credential),
        id: credential.id,
        provider: credential.provider,
        label: credential.label,
        config_dir: credential.config_dir,
        managed: credential.managed,
        env_keys: credential.env_keys,
        account,
        used_by,
        created_at: credential.created_at,
        updated_at: credential.updated_at,
    }
}

fn overview(state: &AppState) -> Result<CredentialsOverview, String> {
    let (credentials, bound) = with_db(state, |conn| {
        Ok((store::list(conn)?, store::bound_role_names(conn)?))
    })?;
    let mut used_by: HashMap<String, Vec<String>> = HashMap::new();
    for (credential_id, role_name) in bound {
        let names = used_by.entry(credential_id).or_default();
        if !names.contains(&role_name) {
            names.push(role_name);
        }
    }
    let providers = ALL_PROVIDERS
        .iter()
        .map(|&provider| ProviderView {
            key: provider.key(),
            label: provider.label(),
            supports_sign_in: provider.config_dir_env().is_some(),
            login_hint: provider.login_hint(),
            suggested_env_keys: provider.suggested_env_keys(),
            default_config_dir: provider
                .default_config_dir()
                .map(|dir| dir.to_string_lossy().into_owned()),
            default_account: provider.default_account_status(),
        })
        .collect();
    let credentials = credentials
        .into_iter()
        .map(|c| {
            let names = used_by.remove(&c.id).unwrap_or_default();
            credential_view(c, names)
        })
        .collect();
    Ok(CredentialsOverview {
        providers,
        credentials,
    })
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCredentialInput {
    provider: String,
    label: String,
    /// An existing config dir to reuse (e.g. one already signed in from a terminal). `None`
    /// creates a fresh Jockey-managed dir when the CLI supports one.
    config_dir: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCredentialInput {
    id: String,
    label: Option<String>,
    #[serde(default)]
    set_env: BTreeMap<String, String>,
    #[serde(default)]
    remove_env: Vec<String>,
}

#[tauri::command]
pub(crate) fn list_agent_credentials_cmd(
    state: State<'_, AppState>,
) -> Result<CredentialsOverview, String> {
    overview(get_state(&state))
}

#[tauri::command]
pub(crate) fn create_agent_credential_cmd(
    state: State<'_, AppState>,
    input: CreateCredentialInput,
) -> Result<CredentialsOverview, String> {
    let state = get_state(&state);
    let provider = CredentialProvider::from_key(&input.provider)
        .ok_or_else(|| format!("unknown agent: {}", input.provider))?;
    let label = input.label.trim().to_string();
    if label.is_empty() {
        return Err("account name is required".to_string());
    }
    let env: BTreeMap<String, String> = input
        .env
        .into_iter()
        .map(|(k, v)| (k.trim().to_string(), v))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    for key in env.keys() {
        validate_env_key(provider, key)?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let existing_dir = input
        .config_dir
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    let (config_dir, managed) = match (provider.config_dir_env(), existing_dir) {
        (None, Some(_)) => {
            return Err(format!(
                "{} cannot use a separate config dir",
                provider.label()
            ))
        }
        (None, None) => {
            if env.is_empty() {
                return Err(format!(
                    "{} accounts need at least one env value",
                    provider.label()
                ));
            }
            (None, false)
        }
        (Some(_), Some(dir)) => {
            let path = PathBuf::from(&dir);
            if !path.is_dir() {
                return Err(format!("not a directory: {dir}"));
            }
            (Some(dir), false)
        }
        (Some(_), None) => {
            let dir = managed_root()?.join(provider.key()).join(&id);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            provider.link_shared_entries(&dir);
            (Some(dir.to_string_lossy().into_owned()), true)
        }
    };

    secrets::save(&id, &env)?;
    let now = crate::now_ms();
    let credential = AgentCredential {
        id: id.clone(),
        provider,
        label,
        config_dir,
        managed,
        env_keys: env.keys().cloned().collect(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = with_db(state, |conn| store::upsert(conn, &credential)) {
        let _ = secrets::delete(&id);
        return Err(e);
    }
    overview(state)
}

#[tauri::command]
pub(crate) fn update_agent_credential_cmd(
    state: State<'_, AppState>,
    input: UpdateCredentialInput,
) -> Result<CredentialsOverview, String> {
    let state = get_state(&state);
    let mut credential = with_db(state, |conn| store::get(conn, &input.id))?
        .ok_or_else(|| "account not found".to_string())?;
    if let Some(label) = input.label.map(|l| l.trim().to_string()) {
        if label.is_empty() {
            return Err("account name is required".to_string());
        }
        credential.label = label;
    }
    if !input.set_env.is_empty() || !input.remove_env.is_empty() {
        let mut env = secrets::load(&credential.id)?;
        for key in &input.remove_env {
            env.remove(key.trim());
        }
        for (key, value) in input.set_env {
            let key = key.trim().to_string();
            validate_env_key(credential.provider, &key)?;
            if value.is_empty() {
                env.remove(&key);
            } else {
                env.insert(key, value);
            }
        }
        if credential.config_dir.is_none() && env.is_empty() {
            return Err(format!(
                "{} accounts need at least one env value",
                credential.provider.label()
            ));
        }
        secrets::save(&credential.id, &env)?;
        credential.env_keys = env.keys().cloned().collect();
    }
    credential.updated_at = crate::now_ms();
    with_db(state, |conn| store::upsert(conn, &credential))?;
    overview(state)
}

/// Removes the profile and its bindings (those roles fall back to the default login). A
/// managed dir is deleted with it; its shared entries are symlinks, which `remove_dir_all`
/// unlinks without following, so the default dir is never touched.
#[tauri::command]
pub(crate) fn delete_agent_credential_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<CredentialsOverview, String> {
    let state = get_state(&state);
    let credential = with_db(state, |conn| store::get(conn, &id))?;
    with_db(state, |conn| store::delete(conn, &id))?;
    secrets::delete(&id)?;
    if let Some(credential) = credential.filter(|c| c.managed) {
        if let Some(dir) = credential.config_dir.as_deref().map(Path::new) {
            if is_managed_path(dir) {
                if let Err(e) = std::fs::remove_dir_all(dir) {
                    eprintln!("[credentials] failed to remove {}: {e}", dir.display());
                }
            }
        }
    }
    overview(state)
}

#[tauri::command]
pub(crate) fn get_role_credentials_cmd(
    state: State<'_, AppState>,
    role_id: String,
) -> Result<Vec<RoleCredentialBinding>, String> {
    let bindings = with_db(get_state(&state), |conn| {
        store::role_bindings(conn, &role_id)
    })?;
    Ok(bindings
        .into_iter()
        .map(|(provider, credential_id)| RoleCredentialBinding {
            provider,
            credential_id,
        })
        .collect())
}

/// `credential_id: None` returns the role to the CLI's default login.
#[tauri::command]
pub(crate) fn set_role_credential_cmd(
    state: State<'_, AppState>,
    role_id: String,
    provider: String,
    credential_id: Option<String>,
) -> Result<(), String> {
    let provider = CredentialProvider::from_key(&provider)
        .ok_or_else(|| format!("unknown agent: {provider}"))?;
    let credential_id = credential_id.filter(|id| !id.trim().is_empty());
    with_db(get_state(&state), |conn| {
        if let Some(id) = credential_id.as_deref() {
            let credential = store::get(conn, id)?.ok_or("account not found")?;
            if credential.provider != provider {
                return Err(format!(
                    "\"{}\" is a {} account",
                    credential.label,
                    credential.provider.label()
                ));
            }
        }
        store::set_binding(
            conn,
            &role_id,
            provider,
            credential_id.as_deref(),
            crate::now_ms(),
        )
    })
}

/// Opens Terminal on the CLI's own sign-in flow, pointed at the profile's config dir. The CLI
/// writes the new login there (and, for Claude, into a keychain item named after that dir),
/// so the default login is left as it was.
#[tauri::command]
pub(crate) fn open_agent_credential_login_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let credential = with_db(get_state(&state), |conn| store::get(conn, &id))?
        .ok_or_else(|| "account not found".to_string())?;
    let command = login_command(&credential)
        .ok_or_else(|| format!("{} has no separate sign-in", credential.provider.label()))?;
    open_in_terminal(&format!("sign in \"{}\"", credential.label), &command)
}

/// Runs `command` in a new Terminal window through a login shell, so the user's PATH applies.
#[cfg(target_os = "macos")]
pub(crate) fn open_in_terminal(label: &str, command: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let script = std::env::temp_dir().join(format!("jockey-{}.command", uuid::Uuid::new_v4()));
    let body = format!(
        "#!/bin/zsh -il\n# Jockey: {}\nrm -f {}\n{command}\n",
        label.replace('\n', " "),
        shell_quote(&script.to_string_lossy()),
    );
    std::fs::write(&script, body).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .args(["-a", "Terminal"])
        .arg(&script)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to open Terminal: {e}"))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn open_in_terminal(_label: &str, command: &str) -> Result<(), String> {
    Err(format!("run this in a terminal: {command}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_command_points_the_cli_at_the_profile_dir() {
        let credential = AgentCredential {
            id: "c1".into(),
            provider: CredentialProvider::Claude,
            label: "Work".into(),
            config_dir: Some("/tmp/it's here".into()),
            managed: true,
            env_keys: vec![],
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(
            login_command(&credential).as_deref(),
            Some(r"CLAUDE_CONFIG_DIR='/tmp/it'\''s here' claude auth login")
        );
    }

    #[test]
    fn env_keys_cannot_override_the_config_dir() {
        assert!(validate_env_key(CredentialProvider::Codex, "OPENAI_API_KEY").is_ok());
        assert!(validate_env_key(CredentialProvider::Codex, "CODEX_HOME").is_err());
        assert!(validate_env_key(CredentialProvider::Codex, "lower").is_err());
        assert!(validate_env_key(CredentialProvider::Codex, "1KEY").is_err());
    }
}
