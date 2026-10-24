mod acp_list;
pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod pi;

use crate::db::app_session_role::save_app_session_role_cli_id;
use crate::db::project::get_project_internal;
use crate::types::{AppSession, AppState, Project};
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const CATALOG_FRESH_FOR_MS: i64 = 5 * 60 * 1000;
/// Catalog scope for the machine-wide scan. Not a project id, so it never collides with one.
const ALL_SCOPE: &str = "*all*";

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
    /// The transcript import copies history from. Empty for conversations only an ACP adapter
    /// lists (no transcript reader for that runtime): importing binds them without history.
    pub(crate) file_path: PathBuf,
    pub(crate) cwd: String,
    pub(crate) title: String,
    pub(crate) created_at: i64,
    pub(crate) last_active_at: i64,
    pub(crate) turn_count: usize,
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
    /// The Jockey session that already holds this conversation, when `imported`.
    pub(crate) app_session_id: Option<String>,
    /// Where importing lands: an existing project, or `None` when one would be created for `cwd`.
    pub(crate) project_id: Option<String>,
    pub(crate) project_name: Option<String>,
}

/// Result of importing from the machine-wide list.
pub(crate) struct CliImport {
    pub(crate) sessions: Vec<AppSession>,
    pub(crate) created_projects: Vec<Project>,
}

fn source_id(runtime_kind: &str, session_id: &str) -> String {
    format!("{runtime_kind}:{session_id}")
}

fn imported_source_ids(state: &AppState) -> HashMap<String, String> {
    crate::db::with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT runtime_kind, external_session_id, id FROM app_sessions
                 WHERE external_session_id IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows
            .filter_map(Result::ok)
            .map(|(runtime_kind, session_id, id)| (source_id(&runtime_kind, &session_id), id))
            .collect::<HashMap<String, String>>())
    })
    .unwrap_or_default()
}

/// Provider conversations Jockey itself started, keyed by provider session id. Their id lives in
/// the session-role tables, not in `app_sessions.external_session_id`, so without this the
/// import picker offered Jockey's own sessions and importing one created a duplicate.
fn jockey_owned_conversations(state: &AppState) -> HashMap<String, String> {
    crate::db::with_db(state, |conn| {
        let mut owned = HashMap::new();
        for sql in [
            "SELECT acp_session_id, app_session_id FROM app_session_role_runtime_configs
             WHERE acp_session_id IS NOT NULL AND trim(acp_session_id) <> ''",
            "SELECT acp_session_id, app_session_id FROM app_session_roles
             WHERE acp_session_id IS NOT NULL AND trim(acp_session_id) <> ''",
        ] {
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for (provider_id, app_session_id) in rows.filter_map(Result::ok) {
                owned.entry(provider_id).or_insert(app_session_id);
            }
        }
        Ok(owned)
    })
    .unwrap_or_default()
}

fn scan_sessions(project_path: &str) -> Vec<DiscoveredSession> {
    let mut sessions = claude::scan_claude_sessions(project_path);
    sessions.extend(codex::scan_codex_sessions(project_path));
    sessions.extend(pi::scan_pi_sessions(project_path));
    acp_list::enrich(&mut sessions, |cwd| cwd_in_project(cwd, project_path));
    finish_scan(sessions)
}

fn scan_all_sessions() -> Vec<DiscoveredSession> {
    let mut sessions = claude::scan_all_claude_sessions();
    sessions.extend(codex::scan_all_codex_sessions());
    sessions.extend(pi::scan_all_pi_sessions());
    acp_list::enrich(&mut sessions, |_| true);
    finish_scan(sessions)
}

fn finish_scan(sessions: Vec<DiscoveredSession>) -> Vec<DiscoveredSession> {
    let mut sessions = deduplicate_sessions(sessions);
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

pub(crate) fn parser_for(runtime_kind: &str) -> Option<fn(&Path) -> Vec<ParsedImportedMessage>> {
    match runtime_kind {
        "claude-code" | "claude-native" => Some(claude::parse_claude_session_messages),
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
                    Some(DiscoveredSession {
                        runtime_kind: match runtime_kind.as_str() {
                            "claude-code" | "claude-native" => "claude-code",
                            "codex-cli" => "codex-cli",
                            "pi-cli" => "pi-cli",
                            "antigravity-cli" => "antigravity-cli",
                            _ => return None,
                        },
                        agent: match agent.as_str() {
                            "Claude Code" => "Claude Code",
                            "Codex" => "Codex",
                            "Pi" => "Pi",
                            "Antigravity" => "Antigravity",
                            _ => return None,
                        },
                        session_id,
                        file_path: PathBuf::from(file_path),
                        cwd,
                        title,
                        created_at,
                        last_active_at,
                        turn_count: turn_count.max(0) as usize,
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

fn importable_from(
    state: &AppState,
    sessions: Vec<DiscoveredSession>,
    resolve: impl Fn(&str) -> Option<(String, String)>,
) -> Vec<ImportableSession> {
    let already = imported_source_ids(state);
    let owned = jockey_owned_conversations(state);
    sessions
        .into_iter()
        .map(|session| {
            let sid = source_id(session.runtime_kind, &session.session_id);
            let app_session_id = already
                .get(&sid)
                .or_else(|| owned.get(&session.session_id))
                .cloned();
            let target = resolve(&session.cwd);
            ImportableSession {
                source_id: sid,
                imported: app_session_id.is_some(),
                app_session_id,
                project_id: target.as_ref().map(|(id, _)| id.clone()),
                project_name: target.map(|(_, name)| name),
                external_session_id: session.session_id,
                runtime_kind: session.runtime_kind.to_string(),
                agent: session.agent.to_string(),
                cwd: session.cwd,
                title: session.title,
                created_at: session.created_at,
                last_active_at: session.last_active_at,
                turn_count: session.turn_count,
            }
        })
        .collect()
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
    let target = (project.id.clone(), project.name.clone());
    Ok(importable_from(
        state,
        cached_sessions(state, project_id)?,
        |_| Some(target.clone()),
    ))
}

/// Every CLI session on the machine, each annotated with the project it would be imported into.
pub(crate) fn scan_all_importable_sessions_internal(
    state: &AppState,
    force_refresh: bool,
) -> Result<Vec<ImportableSession>, String> {
    ensure_all_catalog(state, force_refresh)?;
    let projects = crate::db::project::list_projects_internal(state)?;
    Ok(importable_from(
        state,
        cached_sessions(state, ALL_SCOPE)?,
        |cwd| resolve_project(&projects, cwd).map(|p| (p.id.clone(), p.name.clone())),
    ))
}

fn ensure_all_catalog(state: &AppState, force_refresh: bool) -> Result<(), String> {
    if force_refresh || !catalog_is_fresh(state, ALL_SCOPE)? {
        replace_catalog(state, ALL_SCOPE, &scan_all_sessions())?;
    }
    Ok(())
}

/// The project a working directory belongs to: the one rooted exactly there, else the one with
/// the deepest root containing it. `None` means a project has to be created.
pub(crate) fn resolve_project<'a>(projects: &'a [Project], cwd: &str) -> Option<&'a Project> {
    let deepest = |dir: &Path| {
        projects
            .iter()
            .filter(|p| dir.starts_with(&p.root_path))
            .max_by_key(|p| Path::new(&p.root_path).components().count())
    };
    deepest(Path::new(cwd)).or_else(|| worktree_main_repo(cwd).and_then(|main| deepest(&main)))
}

/// The main checkout a git worktree belongs to. Codex (and other agents) run forks and parallel
/// tasks in worktrees outside the project (`~/.codex/worktrees/<id>/<repo>`); their `.git` file
/// points back at `<main>/.git/worktrees/<name>`, which is what ties them to the project.
pub(crate) fn worktree_main_repo(cwd: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(Path::new(cwd).join(".git")).ok()?;
    let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let (main_git, _) = gitdir.split_once("/.git/worktrees/")?;
    Some(PathBuf::from(main_git))
}

/// Whether a session recorded in `cwd` belongs to the project rooted at `project_root`,
/// counting sub-directories and worktrees of that project.
pub(crate) fn cwd_in_project(cwd: &str, project_root: &str) -> bool {
    Path::new(cwd).starts_with(project_root)
        || worktree_main_repo(cwd).is_some_and(|main| main.starts_with(project_root))
}

fn project_name_for(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(cwd)
        .to_string()
}

/// Import the selected transcripts. `only` is the set of external session ids the user picked;
/// `None` means every transcript found, which is what the old sync-everything button did.
/// Where a provider keeps the transcript of conversation `provider_id`, if it is on disk.
pub(crate) fn locate_transcript(runtime_kind: &str, provider_id: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let file_named = |root: PathBuf| {
        let mut files = Vec::new();
        codex::collect_jsonl_files(&root, &mut files);
        files.into_iter().find(|f| {
            f.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.contains(provider_id))
        })
    };
    match runtime_kind {
        "claude-code" | "claude-native" => {
            let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"));
            // One directory per working directory; the file is named after the session id.
            std::fs::read_dir(config_dir.join("projects"))
                .ok()?
                .flatten()
                .map(|dir| dir.path().join(format!("{provider_id}.jsonl")))
                .find(|path| path.is_file())
        }
        "codex-cli" => file_named(home.join(".codex").join("sessions")),
        "pi-cli" => file_named(home.join(".pi").join("agent").join("sessions")),
        _ => None,
    }
}

/// Keyed by the canonical runtime so a conversation reached under a legacy alias keeps the
/// same message ids and the unique index still dedupes them.
pub(crate) fn transcript_id_prefix(runtime_kind: &str, provider_id: &str) -> String {
    let runtime = crate::runtime_kind::RuntimeKind::from_str(runtime_kind)
        .map_or(runtime_kind, |k| k.runtime_key());
    format!("import:{runtime}:{provider_id}")
}

/// Where to start reading a transcript the session has no cursor for yet. A session whose
/// newest message came from this transcript (a fresh or untouched import) continues after its
/// imported rows. Otherwise Jockey itself has been writing to the conversation and its own
/// record already covers the transcript, so reading starts at the end.
fn baseline_cursor(
    state: &AppState,
    session_id: &str,
    id_prefix: &str,
    transcript_len: usize,
) -> Result<usize, String> {
    let (all, imported) =
        crate::db::app_session::imported_message_counts(state, session_id, id_prefix)?;
    if all == 0 {
        return Ok(0);
    }
    let last = crate::db::app_session::last_message_client_id(state, session_id)?;
    if imported > 0 && last.is_some_and(|id| id.starts_with(&format!("{id_prefix}:"))) {
        return Ok(imported as usize);
    }
    Ok(transcript_len)
}

/// Add the transcript entries past the conversation's cursor to the session and advance the
/// cursor. Returns the messages that were added, shaped like engine messages.
pub(crate) fn sync_transcript(
    state: &AppState,
    session_id: &str,
    conversation: &crate::db::app_session_role::ProviderConversation,
    path: &Path,
) -> Result<Vec<serde_json::Value>, String> {
    let Some(parse) = parser_for(&conversation.runtime_kind) else {
        return Ok(Vec::new());
    };
    let parsed = parse(path);
    let prefix = transcript_id_prefix(&conversation.runtime_kind, &conversation.provider_id);
    let cursor = match conversation.cursor {
        Some(cursor) => cursor,
        None => baseline_cursor(state, session_id, &prefix, parsed.len())?,
    };
    // A transcript shorter than the cursor was rewritten (compaction, manual edit); start
    // following it from its new end rather than re-importing it.
    let start = cursor.min(parsed.len());
    let new: Vec<(String, String, i64)> = parsed[start..]
        .iter()
        .map(|m| (m.role_name.clone(), m.content.clone(), m.created_at))
        .collect();
    let added =
        crate::db::app_session::insert_imported_messages(state, session_id, &prefix, start, &new)?;
    if conversation.cursor != Some(parsed.len()) {
        crate::db::app_session_role::set_transcript_cursor(
            state,
            session_id,
            conversation,
            parsed.len(),
        )?;
    }
    Ok(added
        .into_iter()
        .map(|pos| {
            let (role_name, text, at) = &new[pos];
            serde_json::json!({
                "id": format!("{prefix}:{}", start + pos),
                "roleName": role_name,
                "text": text,
                "at": at,
            })
        })
        .collect())
}

/// Record that everything now in the transcript is already in the session — called after a
/// turn Jockey ran, whose entries the engine recorded itself.
pub(crate) fn mark_transcript_read(
    state: &AppState,
    session_id: &str,
    conversation: &crate::db::app_session_role::ProviderConversation,
    path: &Path,
) -> Result<(), String> {
    let Some(parse) = parser_for(&conversation.runtime_kind) else {
        return Ok(());
    };
    let len = parse(path).len();
    if conversation.cursor == Some(len) {
        return Ok(());
    }
    crate::db::app_session_role::set_transcript_cursor(state, session_id, conversation, len)
}

/// Bring one discovered conversation into `project_id`; returns the Jockey session holding it.
fn import_one(
    state: &AppState,
    project_id: &str,
    session: &DiscoveredSession,
    already: &HashMap<String, String>,
    owned: &HashMap<String, String>,
) -> Result<String, String> {
    // A conversation Jockey started is already a session; hand that one back.
    if !already.contains_key(&source_id(session.runtime_kind, &session.session_id)) {
        if let Some(app_session_id) = owned.get(&session.session_id) {
            let closed = crate::db::app_session::load_session_head(state, app_session_id)?
                .is_some_and(|head| head.closed);
            if closed {
                crate::db::app_session::reopen_app_session_internal(state, app_session_id)?;
            }
            return Ok(app_session_id.clone());
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

    // Same path the session watcher uses: only transcript entries past the conversation's
    // cursor are added, so turns Jockey itself ran are never imported a second time.
    // Propagate rather than swallow: a silent failure here would return the session as if
    // fully imported while it actually has zero messages.
    let conversation =
        crate::db::app_session_role::list_provider_conversations(state, &app_sess.id)?
            .into_iter()
            .find(|c| {
                c.runtime_kind == session.runtime_kind && c.provider_id == session.session_id
            });
    // An ACP-only conversation has no transcript here; the binding alone lets the next turn
    // resume it with `session/load`, and its history stays with the agent.
    if let Some(conversation) = conversation.filter(|_| !session.file_path.as_os_str().is_empty()) {
        sync_transcript(state, &app_sess.id, &conversation, &session.file_path)?;
    }
    Ok(app_sess.id)
}

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

    let already = imported_source_ids(state);
    let owned = jockey_owned_conversations(state);
    let mut imported: Vec<String> = Vec::new();
    for session in cached_sessions(state, project_id)? {
        if let Some(ref selected) = selected {
            let source_id = source_id(session.runtime_kind, &session.session_id);
            if !selected.contains(source_id.as_str()) {
                continue;
            }
        }
        imported.push(import_one(state, project_id, &session, &already, &owned)?);
    }

    // Re-read so the caller gets the persisted title/role plus the transcript messages that
    // were just inserted; the upsert return value predates them.
    crate::db::app_session::get_app_sessions_by_ids(state, &imported)
}

/// Import sessions picked from the machine-wide list, each into the project its cwd resolves
/// to, creating a project (named after the directory) where none covers it.
pub(crate) fn import_cli_sessions_internal(
    state: &AppState,
    source_ids: &[String],
) -> Result<CliImport, String> {
    ensure_all_catalog(state, false)?;
    let selected: HashSet<&str> = source_ids.iter().map(String::as_str).collect();
    let mut projects = crate::db::project::list_projects_internal(state)?;
    let mut created_projects = Vec::new();
    let already = imported_source_ids(state);
    let owned = jockey_owned_conversations(state);
    let mut imported: Vec<String> = Vec::new();
    for session in cached_sessions(state, ALL_SCOPE)? {
        if !selected.contains(source_id(session.runtime_kind, &session.session_id).as_str()) {
            continue;
        }
        let project_id = match resolve_project(&projects, &session.cwd) {
            Some(project) => project.id.clone(),
            None => {
                let project = crate::db::project::create_project_internal(
                    state,
                    project_name_for(&session.cwd),
                    session.cwd.clone(),
                )?;
                let id = project.id.clone();
                // Later sessions from nested directories resolve to this project.
                projects.push(project.clone());
                created_projects.push(project);
                id
            }
        };
        imported.push(import_one(state, &project_id, &session, &already, &owned)?);
    }
    Ok(CliImport {
        sessions: crate::db::app_session::get_app_sessions_by_ids(state, &imported)?,
        created_projects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            session("claude-code", "same", 100, 2),
            session("claude-code", "same", 100, 5),
        ]);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].turn_count, 5);
    }

    fn project(id: &str, root: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            root_path: root.into(),
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        }
    }

    #[test]
    fn cwd_resolves_to_exact_then_deepest_project() {
        let projects = vec![project("outer", "/w/app"), project("inner", "/w/app/pkg")];
        assert_eq!(resolve_project(&projects, "/w/app").unwrap().id, "outer");
        assert_eq!(
            resolve_project(&projects, "/w/app/pkg").unwrap().id,
            "inner"
        );
        assert_eq!(
            resolve_project(&projects, "/w/app/pkg/src").unwrap().id,
            "inner"
        );
        assert_eq!(
            resolve_project(&projects, "/w/app/other").unwrap().id,
            "outer"
        );
        assert!(resolve_project(&projects, "/w/app-two").is_none());
        assert!(resolve_project(&projects, "/elsewhere").is_none());
    }

    #[test]
    fn worktree_sessions_belong_to_their_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("flower");
        let wt = dir.path().join("worktrees").join("077c").join("flower");
        std::fs::create_dir_all(main.join(".git").join("worktrees").join("flower")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}/.git/worktrees/flower\n", main.display()),
        )
        .unwrap();
        let (wt, main) = (wt.to_str().unwrap(), main.to_str().unwrap());
        assert!(cwd_in_project(wt, main));
        assert!(!cwd_in_project(main, wt));
        let projects = vec![project("flower", main)];
        assert_eq!(resolve_project(&projects, wt).unwrap().id, "flower");
    }

    #[test]
    fn project_name_is_the_directory_basename() {
        assert_eq!(project_name_for("/a/b/repo"), "repo");
        assert_eq!(project_name_for("/"), "/");
    }

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool =
            crate::db::DbPool::new(dir.path().join("t.sqlite3"), 2, "PRAGMA foreign_keys = ON;")
                .expect("pool");
        crate::db::init_db(&pool.get().expect("conn")).expect("init");
        AppState {
            db: pool,
            role_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }

    #[test]
    fn importing_from_all_creates_a_project_only_where_none_covers_the_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let known = root.join("known");
        let fresh = root.join("fresh");
        std::fs::create_dir_all(known.join("sub")).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        let existing = crate::db::project::create_project_internal(
            &state,
            "known".into(),
            known.to_string_lossy().to_string(),
        )
        .unwrap();

        let mut sessions = Vec::new();
        for (id, cwd) in [("s1", known.join("sub")), ("s2", fresh.clone())] {
            let transcript = root.join(format!("{id}.jsonl"));
            std::fs::write(&transcript, "").unwrap();
            let mut found = session("claude-code", id, 10, 1);
            found.agent = "Claude Code";
            found.cwd = cwd.to_string_lossy().to_string();
            found.file_path = transcript;
            sessions.push(found);
        }
        replace_catalog(&state, ALL_SCOPE, &sessions).unwrap();

        let listed = scan_all_importable_sessions_internal(&state, false).unwrap();
        let by_id = |id: &str| listed.iter().find(|s| s.external_session_id == id).unwrap();
        assert_eq!(
            by_id("s1").project_id.as_deref(),
            Some(existing.id.as_str())
        );
        assert!(by_id("s2").project_id.is_none());

        let result = import_cli_sessions_internal(
            &state,
            &["claude-code:s1".to_string(), "claude-code:s2".to_string()],
        )
        .unwrap();
        assert_eq!(result.sessions.len(), 2);
        assert_eq!(result.created_projects.len(), 1);
        assert_eq!(result.created_projects[0].name, "fresh");
        let project_of = |ext: &str| {
            result
                .sessions
                .iter()
                .find(|s| s.external_session_id.as_deref() == Some(ext))
                .and_then(|s| s.project_id.clone())
        };
        assert_eq!(project_of("s1").as_deref(), Some(existing.id.as_str()));
        assert_eq!(
            project_of("s2"),
            Some(result.created_projects[0].id.clone())
        );

        let again = scan_all_importable_sessions_internal(&state, false).unwrap();
        assert!(again
            .iter()
            .all(|s| s.imported && s.app_session_id.is_some()));
    }
}
