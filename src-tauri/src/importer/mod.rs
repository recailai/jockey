pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod pi;

use crate::db::app_session_role::save_app_session_role_cli_id;
use crate::db::project::get_project_internal;
use crate::types::{AppSession, AppState};
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const CATALOG_FRESH_FOR_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone)]
pub(crate) struct ParsedImportedMessage {
    pub(crate) role_name: String,
    pub(crate) content: String,
    pub(crate) created_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredSession {
    pub(crate) runtime_kind: &'static str,
    pub(crate) agent: &'static str,
    pub(crate) session_id: String,
    pub(crate) file_path: PathBuf,
    pub(crate) cwd: String,
    pub(crate) title: String,
    pub(crate) created_at: i64,
    pub(crate) last_active_at: i64,
    pub(crate) turn_count: usize,
    pub(crate) parse_messages: fn(&Path) -> Vec<ParsedImportedMessage>,
}

/// A CLI transcript found on disk that the user may choose to pull in. Distinct from an
/// imported `AppSession`: nothing has been written to the database yet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportableSession {
    /// Stable, agent-qualified selection key. The raw id alone belongs to the provider and is
    /// only used to resume that provider after import.
    pub(crate) source_id: String,
    pub(crate) external_session_id: String,
    pub(crate) runtime_kind: String,
    pub(crate) agent: String,
    pub(crate) cwd: String,
    pub(crate) title: String,
    pub(crate) turn_count: usize,
    pub(crate) created_at: i64,
    pub(crate) last_active_at: i64,
    /// Already pulled in — the picker shows it, but selecting it only refreshes timestamps.
    pub(crate) imported: bool,
}

fn source_id(runtime_kind: &str, session_id: &str) -> String {
    format!("{runtime_kind}:{session_id}")
}

fn imported_source_ids(state: &AppState) -> HashSet<String> {
    crate::db::with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT runtime_kind, external_session_id FROM app_sessions
                 WHERE external_session_id IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows
            .filter_map(Result::ok)
            .map(|(runtime_kind, session_id)| source_id(&runtime_kind, &session_id))
            .collect::<HashSet<String>>())
    })
    .unwrap_or_default()
}

fn scan_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    let mut sessions = claude::scan_claude_sessions(project_path);
    sessions.extend(codex::scan_codex_sessions(project_path));
    sessions.extend(pi::scan_pi_sessions(project_path));
    sessions = deduplicate_sessions(sessions);
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at));
    sessions
}

fn deduplicate_sessions(sessions: Vec<DiscoveredSession>) -> Vec<DiscoveredSession> {
    let mut unique = HashMap::<String, DiscoveredSession>::new();
    for session in sessions {
        let key = source_id(session.runtime_kind, &session.session_id);
        let replace = unique.get(&key).is_none_or(|existing| {
            session.last_active_at > existing.last_active_at
                || (session.last_active_at == existing.last_active_at
                    && session.turn_count > existing.turn_count)
        });
        if replace {
            unique.insert(key, session);
        }
    }
    unique.into_values().collect()
}

fn parser_for(runtime_kind: &str) -> Option<fn(&Path) -> Vec<ParsedImportedMessage>> {
    match runtime_kind {
        "claude-native" => Some(claude::parse_claude_session_messages),
        "codex-cli" => Some(codex::parse_codex_session_messages),
        "pi-cli" => Some(pi::parse_pi_session_messages),
        _ => None,
    }
}

fn catalog_is_fresh(state: &AppState, project_id: &str) -> Result<bool, String> {
    let newest = crate::db::with_db(state, |conn| {
        conn.query_row(
            "SELECT max(scanned_at) FROM import_session_catalog_scans WHERE project_id = ?1",
            params![project_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(newest.is_some_and(|at| crate::now_ms().saturating_sub(at) < CATALOG_FRESH_FOR_MS))
}

fn replace_catalog(
    state: &AppState,
    project_id: &str,
    sessions: &[DiscoveredSession],
) -> Result<(), String> {
    let scanned_at = crate::now_ms();
    crate::db::with_db(state, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM import_session_catalog WHERE project_id = ?1",
            params![project_id],
        )
        .map_err(|e| e.to_string())?;
        let mut insert = tx
            .prepare(
                "INSERT INTO import_session_catalog (
                   project_id, source_id, external_session_id, runtime_kind, agent, cwd,
                   title, turn_count, created_at, last_active_at, file_path, scanned_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .map_err(|e| e.to_string())?;
        for session in sessions {
            insert
                .execute(params![
                    project_id,
                    source_id(session.runtime_kind, &session.session_id),
                    session.session_id,
                    session.runtime_kind,
                    session.agent,
                    session.cwd,
                    session.title,
                    session.turn_count as i64,
                    session.created_at,
                    session.last_active_at,
                    session.file_path.to_string_lossy(),
                    scanned_at,
                ])
                .map_err(|e| e.to_string())?;
        }
        drop(insert);
        tx.execute(
            "INSERT INTO import_session_catalog_scans (project_id, scanned_at)
             VALUES (?1, ?2)
             ON CONFLICT(project_id) DO UPDATE SET scanned_at = excluded.scanned_at",
            params![project_id, scanned_at],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })
}

fn cached_sessions(state: &AppState, project_id: &str) -> Result<Vec<DiscoveredSession>, String> {
    crate::db::with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT runtime_kind, agent, external_session_id, file_path, cwd, title,
                        created_at, last_active_at, turn_count
                 FROM import_session_catalog
                 WHERE project_id = ?1
                 ORDER BY last_active_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let sessions = rows
            .filter_map(Result::ok)
            .filter_map(
                |(
                    runtime_kind,
                    agent,
                    session_id,
                    file_path,
                    cwd,
                    title,
                    created_at,
                    last_active_at,
                    turn_count,
                )| {
                    let parse_messages = parser_for(&runtime_kind)?;
                    Some(DiscoveredSession {
                        runtime_kind: match runtime_kind.as_str() {
                            "claude-native" => "claude-native",
                            "codex-cli" => "codex-cli",
                            "pi-cli" => "pi-cli",
                            _ => return None,
                        },
                        agent: match agent.as_str() {
                            "Claude Code" => "Claude Code",
                            "Codex" => "Codex",
                            "Pi" => "Pi",
                            _ => return None,
                        },
                        session_id,
                        file_path: PathBuf::from(file_path),
                        cwd,
                        title,
                        created_at,
                        last_active_at,
                        turn_count: turn_count.max(0) as usize,
                        parse_messages,
                    })
                },
            )
            .collect::<Vec<_>>();
        Ok(sessions)
    })
}

fn refresh_catalog(state: &AppState, project_id: &str, project_path: &str) -> Result<(), String> {
    replace_catalog(state, project_id, &scan_sessions(project_path))
}

/// List what could be imported for a project without writing anything.
pub(crate) fn scan_importable_sessions_internal(
    state: &AppState,
    project_id: &str,
    force_refresh: bool,
) -> Result<Vec<ImportableSession>, String> {
    let project = get_project_internal(state, project_id)?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    if force_refresh || !catalog_is_fresh(state, project_id)? {
        refresh_catalog(state, project_id, &project.root_path)?;
    }
    let already = imported_source_ids(state);
    Ok(cached_sessions(state, project_id)?
        .into_iter()
        .map(|session| ImportableSession {
            source_id: source_id(session.runtime_kind, &session.session_id),
            imported: already.contains(&source_id(session.runtime_kind, &session.session_id)),
            external_session_id: session.session_id,
            runtime_kind: session.runtime_kind.to_string(),
            agent: session.agent.to_string(),
            cwd: session.cwd,
            title: session.title,
            created_at: session.created_at,
            last_active_at: session.last_active_at,
            turn_count: session.turn_count,
        })
        .collect())
}

/// Import the selected transcripts. `only` is the set of external session ids the user picked;
/// `None` means every transcript found, which is what the old sync-everything button did.
pub(crate) fn import_project_sessions_internal(
    state: &AppState,
    project_id: &str,
    only: Option<&[String]>,
) -> Result<Vec<AppSession>, String> {
    let project = get_project_internal(state, project_id)?
        .ok_or_else(|| format!("project not found: {project_id}"))?;

    let selected: Option<HashSet<&str>> = only.map(|ids| ids.iter().map(String::as_str).collect());

    if !catalog_is_fresh(state, project_id)? {
        refresh_catalog(state, project_id, &project.root_path)?;
    }

    let mut imported: Vec<String> = Vec::new();
    for session in cached_sessions(state, project_id)? {
        if let Some(ref selected) = selected {
            let source_id = source_id(session.runtime_kind, &session.session_id);
            if !selected.contains(source_id.as_str()) {
                continue;
            }
        }

        let app_sess = crate::db::app_session::upsert_imported_session(
            state,
            Some(project_id),
            &session.title,
            &session.cwd,
            session.runtime_kind,
            &session.session_id,
            session.created_at,
            session.last_active_at,
        )?;

        // Bind role to CLI session ID for resume
        let _ = save_app_session_role_cli_id(
            state,
            &app_sess.id,
            session.runtime_kind,
            &app_sess.active_role,
            &session.session_id,
        );

        // Populate messages if session messages are currently empty
        let existing_message_count: i64 = crate::db::with_db(state, |conn| {
            conn.query_row(
                "SELECT count(1) FROM app_session_messages WHERE session_id = ?1",
                rusqlite::params![&app_sess.id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap_or(0);

        if existing_message_count == 0 {
            let parsed_msgs = (session.parse_messages)(&session.file_path);
            if !parsed_msgs.is_empty() {
                let msgs_tuple: Vec<(String, String, i64)> = parsed_msgs
                    .into_iter()
                    .map(|m| (m.role_name, m.content, m.created_at))
                    .collect();
                // Propagate rather than swallow: a silent failure here would return the
                // session as if fully imported while it actually has zero messages.
                crate::db::app_session::insert_imported_messages(state, &app_sess.id, &msgs_tuple)?;
            }
        }

        imported.push(app_sess.id);
    }

    // Re-read so the caller gets the persisted title/role plus the transcript messages that
    // were just inserted; the upsert return value predates them.
    crate::db::app_session::get_app_sessions_by_ids(state, &imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_messages(_: &Path) -> Vec<ParsedImportedMessage> {
        Vec::new()
    }

    fn session(
        runtime_kind: &'static str,
        session_id: &str,
        last_active_at: i64,
        turn_count: usize,
    ) -> DiscoveredSession {
        DiscoveredSession {
            runtime_kind,
            agent: "Test Agent",
            session_id: session_id.to_string(),
            file_path: PathBuf::from(format!("/{runtime_kind}-{session_id}.jsonl")),
            cwd: "/project".to_string(),
            title: session_id.to_string(),
            created_at: last_active_at,
            last_active_at,
            turn_count,
            parse_messages: no_messages,
        }
    }

    #[test]
    fn duplicate_source_ids_keep_the_newest_transcript() {
        let sessions = deduplicate_sessions(vec![
            session("codex-cli", "same", 100, 8),
            session("codex-cli", "same", 200, 2),
            session("pi-cli", "same", 200, 2),
        ]);

        assert_eq!(sessions.len(), 2);
        let codex = sessions
            .iter()
            .find(|session| session.runtime_kind == "codex-cli")
            .expect("codex session retained");
        assert_eq!(codex.last_active_at, 200);
    }

    #[test]
    fn duplicate_source_ids_break_timestamp_ties_by_turn_count() {
        let sessions = deduplicate_sessions(vec![
            session("claude-native", "same", 100, 2),
            session("claude-native", "same", 100, 5),
        ]);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].turn_count, 5);
    }
}
