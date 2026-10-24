//! Following provider conversations (docs/architecture/session-sync.md).
//!
//! Two sources of change a view can miss are handled here:
//! - a Jockey turn the view joined late: the shown-session loop announces `turn/progress`
//!   when the running turn's transcript moved, and a view that is not following the live
//!   frames pulls the snapshot (`live_turn`);
//! - a provider conversation driven outside Jockey (a terminal resuming the same session).
//!   A background manager follows *every* bound conversation, open or closed: an OS file
//!   watcher on the provider transcript roots (debounced), a stat sweep as the fallback for
//!   missed events, and a sweep at startup. Appended transcript bytes are imported through
//!   `importer::sync`, announced as `message/added` / `session/changed`, and the provider's
//!   own turn markers drive the "running outside Jockey" state.
//!
//! Following is read-only: it never reserves a session, never starts an agent process, and
//! reads only the bytes appended since each binding's stored position.

use dashmap::DashMap;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use super::events::{self, SessionChange};
use crate::db::app_session_role::ProviderConversation;
use crate::importer::sync::{OwnWindow, SyncCtx};
use crate::types::AppState;

/// Shown-session loop: live preview of a Jockey turn and 1 s latency for the open session.
const TICK: Duration = Duration::from_secs(1);
/// Stat sweep over every followed transcript: the fallback for events the OS watcher missed.
const SWEEP_EVERY: Duration = Duration::from_secs(30);
/// Quiet period that coalesces a burst of file events into one sync.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// A transcript that is not indexed yet (new conversation) is looked up again no sooner.
const INDEX_MISS_REFRESH: Duration = Duration::from_secs(10);
/// Minimum gap between two syncs of a session; while Jockey itself is writing the
/// conversation the gap is longer, since each sync re-reads the turn in progress.
const MIN_SYNC_GAP: Duration = Duration::from_secs(1);
const OWN_SYNC_GAP: Duration = Duration::from_secs(5);
/// For a runtime whose transcript has no turn markers: how long after it last grew a
/// conversation still counts as running elsewhere.
const EXTERNAL_ACTIVE_FOR: Duration = Duration::from_secs(8);
/// A turn open in the transcript counts as running only while the file was written within
/// this long. A CLI that crashed mid-turn leaves its last marker behind forever, and this
/// state refuses input; a tool that stays silent longer than this is shown as idle.
const MARKER_TRUST_MS: i64 = 10 * 60 * 1000;
/// A turn without an end marker in a file quiet this long is stored as finished.
const STALE_OPEN_MS: i64 = MARKER_TRUST_MS;
/// The CLI can flush its last transcript lines after the turn's result frame reached Jockey.
const OWN_WRITE_SETTLE: Duration = Duration::from_millis(3500);
/// A transcript that is not on disk yet (first turn still starting) is looked up again later.
const RELOCATE_AFTER: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Per-session state
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SessionSync {
    files: HashMap<(String, String), SeenFile>,
    /// Spans during which a Jockey turn drove the provider conversation.
    own: Vec<OwnWindow>,
    last_sync: Option<Instant>,
    external_active: bool,
}

struct SeenFile {
    path: Option<PathBuf>,
    looked_up: Instant,
    len: Option<u64>,
    mtime_ms: i64,
    /// From the provider's own turn markers; `None` while no marker has been seen.
    running: Option<bool>,
    grew_at: Option<Instant>,
    live: Option<Live>,
}

impl SeenFile {
    fn new() -> Self {
        Self {
            path: None,
            looked_up: Instant::now() - RELOCATE_AFTER,
            len: None,
            mtime_ms: 0,
            running: None,
            grew_at: None,
            live: None,
        }
    }
}

/// An external turn still in progress: its preview, announced through the live-turn pull.
struct Live {
    turn_id: String,
    message: Value,
    rev: u64,
}

fn sync_state(session_id: &str) -> Arc<Mutex<SessionSync>> {
    static S: OnceLock<DashMap<String, Arc<Mutex<SessionSync>>>> = OnceLock::new();
    S.get_or_init(DashMap::new)
        .entry(session_id.to_string())
        .or_default()
        .clone()
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
        .or_insert_with(SeenFile::new);
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

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// A Jockey turn ran (or is running) within the settle window: the transcript's recent
/// activity is Jockey's own, never "running outside Jockey".
fn own_recent(sync: &SessionSync, now: i64) -> bool {
    sync.own.iter().any(|w| {
        w.end
            .is_none_or(|end| now <= end + OWN_WRITE_SETTLE.as_millis() as i64)
    })
}

fn compute_external_active(sync: &SessionSync, busy: bool) -> bool {
    let now = crate::now_ms();
    if busy || own_recent(sync, now) {
        return false;
    }
    sync.files.values().any(|f| match f.running {
        Some(running) => running && now - f.mtime_ms < MARKER_TRUST_MS,
        None => f
            .grew_at
            .is_some_and(|at| at.elapsed() < EXTERNAL_ACTIVE_FOR),
    })
}

/// Whether the provider conversation is currently being driven outside Jockey, according to
/// the last sync of its transcript (the provider's own turn markers).
pub(crate) fn is_external_active(session_id: &str) -> bool {
    sync_state(session_id)
        .lock()
        .map(|s| s.external_active)
        .unwrap_or(false)
}

/// The external turn in progress, as the live-turn mechanism serves it.
fn external_live(session_id: &str) -> Option<Value> {
    let sync = sync_state(session_id);
    let sync = sync.lock().ok()?;
    sync.files.values().find_map(|f| {
        f.live
            .as_ref()
            .map(|l| json!({ "turnId": l.turn_id, "rev": l.rev, "message": l.message }))
    })
}

/// The turn running now, whoever drives it: a Jockey turn, or one a terminal is running in
/// the provider conversation.
pub(crate) fn live_turn(session_id: &str) -> Option<Value> {
    super::turns::live_turn(session_id).or_else(|| external_live(session_id))
}

// ---------------------------------------------------------------------------
// Own-write bookkeeping
// ---------------------------------------------------------------------------

/// A Jockey turn is about to write to the provider conversation. Transcript items inside the
/// turn's window are recorded by the engine, so the sync skips exactly those.
pub(crate) fn own_turn_started(session_id: &str) {
    if let Ok(mut sync) = sync_state(session_id).lock() {
        let now = crate::now_ms();
        sync.own
            .retain(|w| w.end.is_none_or(|end| now - end < 3_600_000));
        sync.own.push(OwnWindow {
            start: now,
            end: None,
        });
        sync.external_active = false;
    }
}

/// A Jockey turn ended: close its window, and once the CLI has flushed, sync once so
/// anything written outside the window is picked up and the position moves past the turn.
pub(crate) fn own_turn_finished(_state: &AppState, session_id: &str) {
    if let Ok(mut sync) = sync_state(session_id).lock() {
        let now = crate::now_ms();
        if let Some(window) = sync.own.iter_mut().rev().find(|w| w.end.is_none()) {
            window.end = Some(now);
        }
    }
    schedule_sync_after(session_id, OWN_WRITE_SETTLE + Duration::from_millis(200));
}

// ---------------------------------------------------------------------------
// One synchronous pass over a session
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SyncOutcome {
    added: Vec<Value>,
    /// External turns that ended: `(turn id, persisted message)`.
    finished: Vec<(String, Value)>,
    /// External turns that vanished without a stored result.
    aborted: Vec<String>,
    progress: Vec<(String, u64)>,
    /// Provider slots whose warm process no longer matches the conversation on disk.
    stale_slots: Vec<(String, String)>,
    summary_changed: bool,
}

impl SyncOutcome {
    fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.finished.is_empty()
            && self.aborted.is_empty()
            && self.progress.is_empty()
            && !self.summary_changed
    }
}

fn sync_session(state: &AppState, session_id: &str) -> SyncOutcome {
    let mut outcome = SyncOutcome::default();
    let sync = sync_state(session_id);
    let Ok(mut sync) = sync.lock() else {
        return outcome;
    };
    let busy = super::turns::is_busy(session_id);
    let gap = if busy { OWN_SYNC_GAP } else { MIN_SYNC_GAP };
    if sync.last_sync.is_some_and(|at| at.elapsed() < gap) {
        return outcome;
    }
    let conversations = crate::db::app_session_role::list_provider_conversations(state, session_id)
        .unwrap_or_default();
    let now = crate::now_ms();
    let own = sync.own.clone();
    let own_now = own_recent(&sync, now);
    for conversation in conversations {
        let Some(path) = located(&mut sync, &conversation) else {
            continue;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let len = meta.len();
        let modified = mtime_ms(&meta);
        let seen = sync
            .files
            .entry(key(&conversation))
            .or_insert_with(SeenFile::new);
        if seen.len == Some(len) && conversation.position.is_some() {
            continue;
        }
        let grew = seen.len.is_some_and(|l| len > l);
        let ctx = SyncCtx {
            own: &own,
            stale: now - modified >= STALE_OPEN_MS,
        };
        let result = match crate::importer::sync::sync_binding(
            state,
            session_id,
            &conversation,
            &path,
            &ctx,
        ) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("[watch] transcript sync failed for {session_id}: {e}");
                continue;
            }
        };
        if result.reconciled {
            eprintln!(
                "[watch] transcript of {session_id} changed under its position; re-read in full"
            );
        }
        seen.len = Some(len);
        seen.mtime_ms = modified;
        if grew {
            seen.grew_at = Some(Instant::now());
        }
        // While Jockey is writing the conversation its markers are not "elsewhere".
        match result.running {
            Some(_) if own_now || busy => seen.running = Some(false),
            Some(running) => seen.running = Some(running),
            None => {}
        }

        let mut added = result.added;
        match result.open {
            Some((turn_id, message)) => {
                let previous = seen.live.take();
                let rev = match &previous {
                    Some(p) if p.turn_id == turn_id && p.message == message => p.rev,
                    Some(p) if p.turn_id == turn_id => p.rev + 1,
                    _ => 1,
                };
                if previous
                    .as_ref()
                    .is_none_or(|p| p.rev != rev || p.turn_id != turn_id)
                {
                    outcome.progress.push((turn_id.clone(), rev));
                }
                if let Some(p) = previous.filter(|p| p.turn_id != turn_id) {
                    outcome.aborted.push(p.turn_id);
                }
                seen.live = Some(Live {
                    turn_id,
                    message,
                    rev,
                });
            }
            None => {
                if let Some(previous) = seen.live.take() {
                    match added
                        .iter()
                        .position(|m| m["id"] == previous.turn_id.as_str())
                    {
                        Some(pos) => outcome.finished.push((previous.turn_id, added.remove(pos))),
                        None => outcome.aborted.push(previous.turn_id),
                    }
                }
            }
        }
        if !added.is_empty() && !busy {
            outcome.stale_slots.push((
                conversation.role_name.clone(),
                conversation.runtime_kind.clone(),
            ));
        }
        outcome.added.extend(added);
    }
    sync.last_sync = Some(Instant::now());
    let active = compute_external_active(&sync, busy);
    if active != sync.external_active {
        sync.external_active = active;
        outcome.summary_changed = true;
    }
    if !outcome.added.is_empty() || !outcome.finished.is_empty() {
        outcome.summary_changed = true;
    }
    outcome
}

/// Re-evaluates the running state of a session without reading anything (the trust window of
/// a marker can lapse with no file event). Returns whether it changed.
fn reevaluate(session_id: &str) -> bool {
    let sync = sync_state(session_id);
    let Ok(mut sync) = sync.lock() else {
        return false;
    };
    let active = compute_external_active(&sync, super::turns::is_busy(session_id));
    let changed = active != sync.external_active;
    sync.external_active = active;
    changed
}

async fn publish(app: &AppHandle, session_id: &str, outcome: SyncOutcome) {
    for (turn_id, rev) in &outcome.progress {
        events::emit(
            events::TURN_PROGRESS,
            json!({ "appSessionId": session_id, "turnId": turn_id, "rev": rev }),
        );
    }
    for message in &outcome.added {
        events::emit(
            events::MESSAGE_ADDED,
            json!({ "appSessionId": session_id, "message": message }),
        );
    }
    for (turn_id, message) in &outcome.finished {
        events::emit(
            events::TURN_FINISHED,
            json!({
                "appSessionId": session_id,
                "turnId": turn_id,
                "ok": true,
                "cancelled": false,
                "messages": [message],
            }),
        );
    }
    for turn_id in &outcome.aborted {
        events::emit(
            events::TURN_FINISHED,
            json!({
                "appSessionId": session_id,
                "turnId": turn_id,
                "ok": false,
                "cancelled": true,
                "messages": [],
            }),
        );
    }
    // The next Jockey turn must resume from the provider's record, which now holds turns the
    // warm process never saw.
    for (role, runtime) in &outcome.stale_slots {
        let _ = crate::acp::reset_session(runtime, role, Some(session_id)).await;
    }
    if outcome.summary_changed {
        events::session_changed(&app.state::<AppState>(), session_id, SessionChange::Updated);
    }
}

async fn run_session(app: &AppHandle, session_id: &str) {
    let state = app.state::<AppState>().clone_refs();
    let sid = session_id.to_string();
    let Ok(outcome) = tokio::task::spawn_blocking(move || sync_session(&state, &sid)).await else {
        return;
    };
    if !outcome.is_empty() {
        publish(app, session_id, outcome).await;
    }
}

// ---------------------------------------------------------------------------
// Background manager
// ---------------------------------------------------------------------------

enum Job {
    Session(String),
    Paths(Vec<PathBuf>),
    Sweep { full: bool },
}

static QUEUE: OnceLock<UnboundedSender<Job>> = OnceLock::new();

/// Ask the background manager to sync one session soon (a no-op before it started).
pub(crate) fn schedule_sync(session_id: &str) {
    if let Some(queue) = QUEUE.get() {
        let _ = queue.send(Job::Session(session_id.to_string()));
    }
}

fn schedule_sync_after(session_id: &str, delay: Duration) {
    let Some(queue) = QUEUE.get().cloned() else {
        return;
    };
    let session_id = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let _ = queue.send(Job::Session(session_id));
    });
}

/// Transcript path → the sessions that follow it.
#[derive(Default)]
struct Index {
    by_path: HashMap<PathBuf, Vec<String>>,
    stats: HashMap<PathBuf, (u64, i64)>,
    sessions: HashSet<String>,
    misses: HashSet<PathBuf>,
    built_at: Option<Instant>,
}

fn normalize(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn build_index(state: &AppState) -> Index {
    let mut index = Index::default();
    let conversations =
        crate::db::app_session_role::list_all_provider_conversations(state).unwrap_or_default();
    let wanted: Vec<(String, String)> = conversations
        .iter()
        .map(|(_, c)| (c.runtime_kind.clone(), c.provider_id.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let paths = crate::importer::locate_many(&wanted);
    for (session_id, conversation) in conversations {
        index.sessions.insert(session_id.clone());
        if let Some(path) = paths.get(&key(&conversation)) {
            let path = normalize(path.clone());
            let owners = index.by_path.entry(path).or_default();
            if !owners.contains(&session_id) {
                owners.push(session_id);
            }
        }
    }
    index.built_at = Some(Instant::now());
    index
}

fn start_fs_watcher(tx: UnboundedSender<Job>) -> Option<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        let paths: Vec<PathBuf> = event
            .paths
            .into_iter()
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("jsonl" | "zstd")
                )
            })
            .collect();
        if !paths.is_empty() {
            let _ = tx.send(Job::Paths(paths));
        }
    })
    .map_err(|e| eprintln!("[watch] file watcher unavailable, sweeping instead: {e}"))
    .ok()?;
    let mut any = false;
    for root in crate::importer::transcript_roots() {
        match watcher.watch(&root, RecursiveMode::Recursive) {
            Ok(()) => any = true,
            Err(e) => eprintln!("[watch] cannot watch {}: {e}", root.display()),
        }
    }
    any.then_some(watcher)
}

/// Start following every bound conversation. Called once at setup, after the state exists.
pub(crate) fn start(app: AppHandle) {
    let (tx, mut rx) = unbounded_channel::<Job>();
    if QUEUE.set(tx.clone()).is_err() {
        return;
    }
    let watcher = start_fs_watcher(tx.clone());
    let ticker = tx.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            if ticker.send(Job::Sweep { full: false }).is_err() {
                break;
            }
        }
    });
    let _ = tx.send(Job::Sweep { full: true });

    tauri::async_runtime::spawn(async move {
        // Held for the manager's lifetime: dropping it stops the OS watch.
        let _watcher = watcher;
        let mut index = Index::default();
        while let Some(first) = rx.recv().await {
            let mut sessions: Vec<String> = Vec::new();
            let mut paths: Vec<PathBuf> = Vec::new();
            let mut sweep: Option<bool> = None;
            let mut absorb = |job: Job| match job {
                Job::Session(sid) => sessions.push(sid),
                Job::Paths(p) => paths.extend(p),
                Job::Sweep { full } => sweep = Some(sweep.unwrap_or(false) || full),
            };
            let debounced = matches!(first, Job::Paths(_));
            absorb(first);
            if debounced {
                tokio::time::sleep(DEBOUNCE).await;
            }
            while let Ok(job) = rx.try_recv() {
                absorb(job);
            }

            let state = app.state::<AppState>().clone_refs();
            let needs_index = sweep.is_some()
                || index.built_at.is_none()
                || (!paths.is_empty()
                    && paths.iter().any(|p| {
                        !index.by_path.contains_key(&normalize(p.clone()))
                            && !index.misses.contains(p)
                    })
                    && index
                        .built_at
                        .is_none_or(|at| at.elapsed() >= INDEX_MISS_REFRESH));
            if needs_index {
                let rebuilt = tokio::task::spawn_blocking({
                    let state = state.clone_refs();
                    move || build_index(&state)
                })
                .await;
                if let Ok(rebuilt) = rebuilt {
                    let stats = std::mem::take(&mut index.stats);
                    index = rebuilt;
                    index.stats = stats;
                }
            }

            let mut run: Vec<String> = sessions;
            for path in paths {
                match index.by_path.get(&normalize(path.clone())) {
                    Some(owners) => run.extend(owners.iter().cloned()),
                    None => {
                        index.misses.insert(path);
                        if index.misses.len() > 2000 {
                            index.misses.clear();
                        }
                    }
                }
            }
            if let Some(full) = sweep {
                if full {
                    run.extend(index.sessions.iter().cloned());
                } else {
                    for (path, owners) in &index.by_path {
                        let Ok(meta) = std::fs::metadata(path) else {
                            continue;
                        };
                        let stat = (meta.len(), mtime_ms(&meta));
                        if index.stats.insert(path.clone(), stat) != Some(stat) {
                            run.extend(owners.iter().cloned());
                        }
                    }
                }
            }
            let mut seen = HashSet::new();
            run.retain(|sid| seen.insert(sid.clone()));
            for sid in &run {
                run_session(&app, sid).await;
            }
            if sweep.is_some() {
                for sid in index.sessions.iter().filter(|s| !seen.contains(*s)) {
                    if reevaluate(sid) {
                        events::session_changed(
                            &app.state::<AppState>(),
                            sid,
                            SessionChange::Updated,
                        );
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// The shown session
// ---------------------------------------------------------------------------

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

/// Start following `session_id` for one more view. Returns the running turn's snapshot, so a
/// view opened mid-turn can render it right away.
pub(crate) fn watch(app: &AppHandle, session_id: &str) -> Option<Value> {
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
    live_turn(session_id)
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

/// The shown session: announces a Jockey turn's progress and syncs at 1 s latency (the
/// background manager covers it at file-event latency, this keeps the stat check on a clock
/// for providers whose events are coalesced late).
fn spawn_loop(app: AppHandle, session_id: String, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        let mut last_rev: Option<(String, u64)> = None;
        loop {
            tick.tick().await;
            if !is_current(&session_id, generation) {
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
            run_session(&app, &session_id).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(running: Option<bool>, mtime_ms: i64) -> SeenFile {
        let mut file = SeenFile::new();
        file.running = running;
        file.mtime_ms = mtime_ms;
        file
    }

    fn sync_with(files: Vec<SeenFile>) -> SessionSync {
        let mut sync = SessionSync::default();
        for (i, file) in files.into_iter().enumerate() {
            sync.files
                .insert(("claude-code".into(), format!("p{i}")), file);
        }
        sync
    }

    #[test]
    fn status_follows_the_provider_markers_not_the_clock() {
        let now = crate::now_ms();
        // An open turn is running however long the tool has been silent (within the trust
        // window), and a finished one is idle even though it was just written.
        let open = sync_with(vec![seen(Some(true), now - 120_000)]);
        assert!(compute_external_active(&open, false));
        let closed = sync_with(vec![seen(Some(false), now)]);
        assert!(!compute_external_active(&closed, false));
        // A crashed CLI's last marker stops counting once the file is long quiet.
        let abandoned = sync_with(vec![seen(Some(true), now - MARKER_TRUST_MS - 1000)]);
        assert!(!compute_external_active(&abandoned, false));
        // Jockey's own turn never reads as running elsewhere.
        assert!(!compute_external_active(&open, true));
    }

    #[test]
    fn markerless_runtimes_fall_back_to_recent_growth() {
        let mut file = seen(None, 0);
        file.grew_at = Some(Instant::now());
        assert!(compute_external_active(&sync_with(vec![file]), false));
        let mut old = seen(None, 0);
        old.grew_at = Some(Instant::now() - EXTERNAL_ACTIVE_FOR * 2);
        assert!(!compute_external_active(&sync_with(vec![old]), false));
        assert!(!compute_external_active(
            &sync_with(vec![seen(None, 0)]),
            false
        ));
    }

    #[test]
    fn a_recent_jockey_turn_masks_the_running_state() {
        let now = crate::now_ms();
        let mut sync = sync_with(vec![seen(Some(true), now)]);
        sync.own.push(OwnWindow {
            start: now - 5000,
            end: Some(now - 1000),
        });
        assert!(!compute_external_active(&sync, false));
        sync.own[0].end = Some(now - 60_000);
        assert!(compute_external_active(&sync, false));
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

    #[test]
    fn own_windows_open_on_start_and_close_on_finish() {
        let sid = "own-window-test";
        own_turn_started(sid);
        assert!(own_recent(
            &sync_state(sid).lock().unwrap(),
            crate::now_ms()
        ));
        // `own_turn_finished` needs no queue to close the window.
        let sync = sync_state(sid);
        {
            let mut guard = sync.lock().unwrap();
            let now = crate::now_ms();
            guard.own.last_mut().unwrap().end = Some(now - 60_000);
        }
        assert!(!own_recent(&sync.lock().unwrap(), crate::now_ms()));
    }
}
