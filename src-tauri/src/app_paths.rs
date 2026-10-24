//! Where this build keeps its state.
//!
//! Debug builds (`pnpm tauri dev`) must never share state with the installed app: the
//! SQLite database (engine recovery resets "running" rows on startup), the companion token,
//! and the built-in Jockey MCP registration (which records the live bridge port) are all
//! written at startup, so a dev launch against the release dir would break the running app.

use std::path::{Path, PathBuf};

pub(crate) const DATA_DIR_ENV: &str = "JOCKEY_DATA_DIR";
const DEV_DIR_SUFFIX: &str = ".dev";
pub(crate) const DB_FILE: &str = "jockey.sqlite3";

pub(crate) fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

/// `release_dir` is the platform app-local data dir for the bundle identifier.
pub(crate) fn resolve_data_dir(release_dir: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if is_dev_build() {
        dev_dir_for(release_dir)
    } else {
        release_dir.to_path_buf()
    }
}

fn dev_dir_for(release_dir: &Path) -> PathBuf {
    let mut name = release_dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "jockey".into());
    name.push(DEV_DIR_SUFFIX);
    release_dir.with_file_name(name)
}

/// First dev launch starts from a snapshot of the installed app's database so there are
/// roles and projects to test with. `VACUUM INTO` over a read-only connection is a
/// consistent copy that takes no write lock, so the installed app can keep running.
pub(crate) fn seed_dev_database(release_dir: &Path, data_dir: &Path) {
    if !is_dev_build() || data_dir == release_dir {
        return;
    }
    let target = data_dir.join(DB_FILE);
    let source = release_dir.join(DB_FILE);
    if target.exists() || !source.exists() {
        return;
    }
    let result = rusqlite::Connection::open_with_flags(
        &source,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|conn| conn.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()]));
    match result {
        Ok(_) => eprintln!(
            "[app_paths] dev database seeded from {}",
            source.to_string_lossy()
        ),
        Err(e) => {
            eprintln!("[app_paths] dev database snapshot failed, starting empty: {e}");
            let _ = std::fs::remove_file(&target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_dir_is_a_sibling_of_the_release_dir() {
        let release = Path::new("/Users/me/Library/Application Support/com.jockey.app");
        assert_eq!(
            dev_dir_for(release),
            PathBuf::from("/Users/me/Library/Application Support/com.jockey.app.dev")
        );
    }
}
