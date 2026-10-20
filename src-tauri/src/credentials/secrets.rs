//! Env values for a credential profile (API keys, tokens), kept in the OS keychain under a
//! Jockey-owned service so they never land in the SQLite file. The items belong to Jockey
//! alone; no CLI's own keychain entry is read or written here.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use dashmap::DashMap;

fn service() -> &'static str {
    // Dev builds start from a copy of the release database, so the same profile ids exist in
    // both; a separate service keeps a dev edit from overwriting the installed app's secret.
    if crate::app_paths::is_dev_build() {
        "Jockey Agent Credentials (dev)"
    } else {
        "Jockey Agent Credentials"
    }
}

/// Every turn resolves its launch env, and a keychain read can cost a prompt on unsigned
/// builds, so values are read once per process and kept current by `save`/`delete`.
fn cache() -> &'static DashMap<String, BTreeMap<String, String>> {
    static CACHE: OnceLock<DashMap<String, BTreeMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

fn entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service(), id).map_err(|e| format!("keychain unavailable: {e}"))
}

pub(crate) fn load(id: &str) -> Result<BTreeMap<String, String>, String> {
    if let Some(values) = cache().get(id) {
        return Ok(values.clone());
    }
    let values = match entry(id)?.get_password() {
        Ok(json) => serde_json::from_str(&json)
            .map_err(|e| format!("credential secret is unreadable: {e}"))?,
        Err(keyring::Error::NoEntry) => BTreeMap::new(),
        Err(e) => return Err(format!("failed to read credential secret: {e}")),
    };
    cache().insert(id.to_string(), values.clone());
    Ok(values)
}

pub(crate) fn save(id: &str, values: &BTreeMap<String, String>) -> Result<(), String> {
    let entry = entry(id)?;
    if values.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(format!("failed to clear credential secret: {e}")),
        }
    } else {
        let json = serde_json::to_string(values).map_err(|e| e.to_string())?;
        entry
            .set_password(&json)
            .map_err(|e| format!("failed to store credential secret: {e}"))?;
    }
    cache().insert(id.to_string(), values.clone());
    Ok(())
}

pub(crate) fn delete(id: &str) -> Result<(), String> {
    save(id, &BTreeMap::new())?;
    cache().remove(id);
    Ok(())
}
