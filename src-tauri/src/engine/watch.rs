//! Following a session for the views that show it (docs/architecture/session-sync.md).
//!
//! "Notify, then pull", for both sources of change a view can miss:
//! - a Jockey turn the view joined late: the loop announces `turn/progress` when the running
//!   turn's transcript moved, and a view that is not following the live frames pulls the
//!   snapshot (`turns::live_turn`);
//! - the provider conversation driven outside Jockey (a terminal resuming the same session):
//!   the loop notices the provider's transcript growing, adds the new entries through the
//!   importer's cursor (`importer::sync_transcript`) and announces them as `message/added`.
//!
//! Watching is read-only: it never reserves the session, never starts an agent process, and
//! costs nothing for sessions nobody is viewing.

use dashmap::DashMap;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use super::events::{self, SessionChange};
use crate::db::app_session_role::ProviderConversation;
use crate::types::AppState;

const TICK: Duration = Duration::from_secs(1);
/// How long after its transcript last grew a conversation still counts as running elsewhere.
const EXTERNAL_ACTIVE_FOR: Duration = Duration::from_secs(8);
/// Without a watcher there is no live tick, so an unwatched session counts as driven elsewhere
/// for longer: a CLI conversation sits idle between turns.
const UNWATCHED_ACTIVE_FOR: Duration = Duration::from_secs(10 * 60);
/// The CLI can flush its last transcript lines after the turn's result frame reached Jockey.
const OWN_WRITE_SETTLE: Duration = Duration::from_secs(2);
/// A transcript that is not on disk yet (first turn still starting) is looked up again later.
const RELOCATE_AFTER: Duration = Duration::from_secs(15);

#[derive(Default)]
struct SessionSync {
    files: HashMap<(String, String), SeenFile>,
    /// Set while Jockey itself is (or just was) writing to the provider conversation; the
    /// transcript growth it causes is recorded by the engine, not imported.
    own_write: bool,
    last_external: Option<Instant>,
    external_active: bool,
}

struct SeenFile {
    path: Option<PathBuf>,
    looked_up: Instant,
    len: Option<u64>,
}

/// Per watched session: how many views follow it, and which loop is the current one. A
/// session released and watched again before its old loop noticed gets a new loop; the old
/// one sees the generation change and exits instead of running beside it.
#[derive(Clone, Copy)]
struct Watchers {
    refs: usize,
    generation: u64,
}

fn watchers() -> &'static DashMap<String, Watchers> {
    static W: OnceLock<DashMap<String, Watchers>> = OnceLock::new();
    W.get_or_init(DashMap::new)
}

fn sync_state(session_id: &str) -> Arc<Mutex<SessionSync>> {
    static S: OnceLock<DashMap<String, Arc<Mutex<SessionSync>>>> = OnceLock::new();
    S.get_or_init(DashMap::new)
        .entry(session_id.to_string())
        .or_default()
        .clone()
}

/// Start following `session_id` for one more view. Returns the running turn's snapshot, so a
/// view opened mid-turn can render it right away.
pub(crate) fn watch(app: &AppHandle, session_id: &str) -> Option<serde_json::Value> {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let started = {
        let mut entry = watchers()
            .entry(session_id.to_string())
            .or_insert(Watchers {
                refs: 0,
                generation: 0,
            });
        entry.refs += 1;
        (entry.refs == 1).then(|| {
            entry.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            entry.generation
        })
    };
    if let Some(generation) = started {
        spawn_loop(app.clone(), session_id.to_string(), generation);
    }
    super::turns::live_turn(session_id)
}

pub(crate) fn unwatch(session_id: &str) {
    watchers().remove_if_mut(session_id, |_, w| {
        w.refs = w.refs.saturating_sub(1);
        w.refs == 0
    });
}

fn is_current(session_id: &str, generation: u64) -> bool {
    watchers()
        .get(session_id)
        .is_some_and(|w| w.refs > 0 && w.generation == generation)
}

/// Whether the provider conversation is currently being driven outside Jockey. Only known for
/// sessions someone is watching.
pub(crate) fn is_external_active(session_id: &str) -> bool {
    sync_state(session_id)
        .lock()
        .map(|s| s.external_active)
        .unwrap_or(false)
}

/// For sessions nobody is watching (e.g. archived ones): whether one of its provider
/// conversations was written to recently, i.e. the agent is being driven outside Jockey.
pub(crate) fn provider_recently_written(state: &AppState, session_id: &str) -> bool {
    if super::turns::is_busy(session_id) {
        return false;
    }
    crate::db::app_session_role::list_provider_conversations(state, session_id)
        .unwrap_or_default()
        .iter()
        .filter_map(|c| crate::importer::locate_transcript(&c.runtime_kind, &c.provider_id))
        .filter_map(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .any(|at| at.elapsed().is_ok_and(|age| age < UNWATCHED_ACTIVE_FOR))
}

/// A Jockey turn is about to write to the provider conversation. Waits for a sync in progress,
/// so that sync cannot read the turn's own entries as external ones.
pub(crate) fn own_turn_started(session_id: &str) {
    if let Ok(mut sync) = sync_state(session_id).lock() {
        sync.own_write = true;
    }
}

/// A Jockey turn ended: once the CLI has flushed, mark the transcript read up to its end, so
/// neither the watcher nor a later import adds the turn a second time.
pub(crate) fn own_turn_finished(state: &AppState, session_id: &str) {
    let state = state.clone_refs();
    let session_id = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(OWN_WRITE_SETTLE).await;
        let _ = tokio::task::spawn_blocking(move || {
            let sync = sync_state(&session_id);
            let Ok(mut sync) = sync.lock() else { return };
            // Another turn may have begun during the settle; it will mark the end itself.
            if super::turns::is_busy(&session_id) {
                return;
            }
            let conversations =
                crate::db::app_session_role::list_provider_conversations(&state, &session_id)
                    .unwrap_or_default();
            for conversation in conversations {
                let Some(path) = located(&mut sync, &conversation) else {
                    continue;
                };
                if let Err(e) =
                    crate::importer::mark_transcript_read(&state, &session_id, &conversation, &path)
                {
                    eprintln!("[watch] cursor update failed for {session_id}: {e}");
                }
                let len = std::fs::metadata(&path).ok().map(|m| m.len());
                if let Some(seen) = sync.files.get_mut(&key(&conversation)) {
                    seen.len = len;
                }
            }
            sync.own_write = false;
        })
        .await;
    });
}

fn key(conversation: &ProviderConversation) -> (String, String) {
    (
        conversation.runtime_kind.clone(),
        conversation.provider_id.clone(),
    )
}

fn located(sync: &mut SessionSync, conversation: &ProviderConversation) -> Option<PathBuf> {
    let seen = sync
        .files
        .entry(key(conversation))
        .or_insert_with(|| SeenFile {
            path: None,
            looked_up: Instant::now() - RELOCATE_AFTER,
            len: None,
        });
    if seen.path.as_ref().is_none_or(|p| !p.is_file()) && seen.looked_up.elapsed() >= RELOCATE_AFTER
    {
        seen.looked_up = Instant::now();
        seen.path = crate::importer::locate_transcript(
            &conversation.runtime_kind,
            &conversation.provider_id,
        );
    }
    seen.path.clone()
}

#[derive(Default)]
struct SyncOutcome {
    added: Vec<serde_json::Value>,
    /// Provider slots whose warm process no longer matches the conversation on disk.
    stale_slots: Vec<(String, String)>,
    summary_changed: bool,
}

fn sync_once(state: &AppState, session_id: &str) -> SyncOutcome {
    let mut outcome = SyncOutcome::default();
    let sync = sync_state(session_id);
    let Ok(mut sync) = sync.lock() else {
        return outcome;
    };
    if sync.own_write || super::turns::is_busy(session_id) {
        return outcome;
    }
    let conversations = crate::db::app_session_role::list_provider_conversations(state, session_id)
        .unwrap_or_default();
    for conversation in conversations {
        let Some(path) = located(&mut sync, &conversation) else {
            continue;
        };
        let Some(len) = std::fs::metadata(&path).ok().map(|m| m.len()) else {
            continue;
        };
        let seen_len = sync.files.get(&key(&conversation)).and_then(|f| f.len);
        if seen_len == Some(len) && conversation.cursor.is_some() {
            continue;
        }
        if seen_len.is_some_and(|seen| len > seen) {
            sync.last_external = Some(Instant::now());
        }
        match crate::importer::sync_transcript(state, session_id, &conversation, &path) {
            Ok(added) => {
                if !added.is_empty() {
                    outcome.stale_slots.push((
                        conversation.role_name.clone(),
                        conversation.runtime_kind.clone(),
                    ));
                    outcome.summary_changed = true;
                    outcome.added.extend(added);
                }
                if let Some(seen) = sync.files.get_mut(&key(&conversation)) {
                    seen.len = Some(len);
                }
            }
            Err(e) => eprintln!("[watch] transcript sync failed for {session_id}: {e}"),
        }
    }
    let active = sync
        .last_external
        .is_some_and(|at| at.elapsed() < EXTERNAL_ACTIVE_FOR);
    if active != sync.external_active {
        sync.external_active = active;
        outcome.summary_changed = true;
    }
    outcome
}

fn spawn_loop(app: AppHandle, session_id: String, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        let mut last_rev: Option<(String, u64)> = None;
        loop {
            tick.tick().await;
            if !is_current(&session_id, generation) {
                // Unwatched, nobody can tell whether it is still running elsewhere; a stale
                // `true` would refuse input forever.
                if !watchers().contains_key(&session_id) {
                    if let Ok(mut sync) = sync_state(&session_id).lock() {
                        sync.external_active = false;
                        sync.last_external = None;
                    }
                }
                break;
            }
            let rev = super::turns::live_rev(&session_id);
            if let Some((turn_id, rev)) = rev.as_ref().filter(|r| Some(*r) != last_rev.as_ref()) {
                events::emit(
                    events::TURN_PROGRESS,
                    json!({ "appSessionId": session_id, "turnId": turn_id, "rev": rev }),
                );
            }
            last_rev = rev;
            if last_rev.is_some() {
                continue;
            }

            let state = app.state::<AppState>().clone_refs();
            let sid = session_id.clone();
            let Ok(outcome) = tokio::task::spawn_blocking(move || sync_once(&state, &sid)).await
            else {
                continue;
            };
            for message in &outcome.added {
                events::emit(
                    events::MESSAGE_ADDED,
                    json!({ "appSessionId": session_id, "message": message }),
                );
            }
            // The next Jockey turn must resume from the provider's record, which now holds
            // turns the warm process never saw.
            for (role, runtime) in &outcome.stale_slots {
                let _ = crate::acp::reset_session(runtime, role, Some(&session_id)).await;
            }
            if outcome.summary_changed {
                events::session_changed(
                    &app.state::<AppState>(),
                    &session_id,
                    SessionChange::Updated,
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_db, DbPool};
    use std::io::Write;

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(dir.path().join("t.sqlite3"), 2, "PRAGMA foreign_keys = ON;")
            .expect("pool");
        init_db(&pool.get().expect("conn")).expect("init");
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    fn append(path: &std::path::Path, entries: &[(&str, &str)]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for (kind, text) in entries {
            let line = json!({ "type": kind, "message": { "role": kind, "content": text } });
            writeln!(file, "{line}").unwrap();
        }
    }

    fn conversation(state: &AppState, sid: &str) -> ProviderConversation {
        crate::db::app_session_role::list_provider_conversations(state, sid)
            .unwrap()
            .pop()
            .expect("conversation")
    }

    fn texts(added: &[serde_json::Value]) -> Vec<&str> {
        added.iter().map(|m| m["text"].as_str().unwrap()).collect()
    }

    #[test]
    fn transcript_sync_follows_external_turns_but_never_jockeys_own() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let transcript = dir.path().join("pid.jsonl");
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("s"),
            None,
            Some("claude-native"),
            None,
            None,
        )
        .unwrap();
        // A Jockey-run conversation: the engine already recorded the first exchange.
        crate::db::app_session::save_app_message_internal(
            &state,
            &session.id,
            Some("u1"),
            "user",
            "hello",
            None,
            None,
        )
        .unwrap();
        crate::db::app_session_role::save_app_session_role_cli_id(
            &state,
            &session.id,
            "claude-native",
            "Developer",
            "pid",
        )
        .unwrap();
        append(&transcript, &[("user", "hello"), ("assistant", "hi there")]);

        // First sight: Jockey's record already covers the transcript.
        let sync = |state: &AppState| {
            crate::importer::sync_transcript(
                state,
                &session.id,
                &conversation(state, &session.id),
                &transcript,
            )
            .unwrap()
        };
        assert!(sync(&state).is_empty());
        assert_eq!(conversation(&state, &session.id).cursor, Some(2));

        // A terminal resumed the conversation and ran a turn.
        append(
            &transcript,
            &[("user", "from terminal"), ("assistant", "terminal reply")],
        );
        let added = sync(&state);
        assert_eq!(texts(&added), ["from terminal", "terminal reply"]);
        assert_eq!(added[0]["id"], "import:claude-code:pid:2");
        assert!(sync(&state).is_empty(), "idempotent");

        // Jockey's own next turn lands in the transcript too; marking it read keeps it out.
        append(
            &transcript,
            &[("user", "from jockey"), ("assistant", "jockey reply")],
        );
        crate::importer::mark_transcript_read(
            &state,
            &session.id,
            &conversation(&state, &session.id),
            &transcript,
        )
        .unwrap();
        assert!(sync(&state).is_empty());

        // A different provider conversation starts from its own beginning.
        crate::db::app_session_role::save_app_session_role_cli_id(
            &state,
            &session.id,
            "claude-native",
            "Developer",
            "pid-2",
        )
        .unwrap();
        assert_eq!(conversation(&state, &session.id).cursor, None);
    }

    #[test]
    fn untouched_import_continues_after_its_imported_rows() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir);
        let transcript = dir.path().join("pid.jsonl");
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("s"),
            None,
            Some("claude-native"),
            None,
            None,
        )
        .unwrap();
        crate::db::app_session_role::save_app_session_role_cli_id(
            &state,
            &session.id,
            "claude-native",
            "Developer",
            "pid",
        )
        .unwrap();
        append(&transcript, &[("user", "a"), ("assistant", "b")]);
        let sync = |state: &AppState| {
            crate::importer::sync_transcript(
                state,
                &session.id,
                &conversation(state, &session.id),
                &transcript,
            )
            .unwrap()
        };
        assert_eq!(
            texts(&sync(&state)),
            ["a", "b"],
            "an empty session imports everything"
        );

        // Imported before cursors existed, then continued in the terminal.
        crate::db::with_db(&state, |conn| {
            conn.execute(
                "UPDATE app_session_role_runtime_configs SET transcript_cursor = NULL",
                [],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        append(&transcript, &[("user", "c")]);
        assert_eq!(texts(&sync(&state)), ["c"]);
    }

    #[test]
    fn watch_refcounts_per_session() {
        let sid = "watch-refcount-test";
        watchers().insert(
            sid.to_string(),
            Watchers {
                refs: 2,
                generation: 7,
            },
        );
        unwatch(sid);
        assert!(is_current(sid, 7));
        assert!(!is_current(sid, 6), "an older loop is not the current one");
        unwatch(sid);
        assert!(!is_current(sid, 7));
        unwatch(sid);
        assert!(!is_current(sid, 7), "extra releases are harmless");
    }
}
