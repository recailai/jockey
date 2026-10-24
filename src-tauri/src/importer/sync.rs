//! Bringing one provider conversation's transcript into its app session, incrementally.
//!
//! The binding stores `(offset, fingerprint)`: the byte offset after the last fully handled
//! line and a fingerprint of the file around it. A sync verifies the fingerprint (two small
//! reads), reads only the appended bytes, and stores the turns it finished under provider-native
//! ids (`import:{runtime}:{provider id}:u:<uuid>` / `:t:<uuid>`), so ids survive parser
//! changes and a repeated read is an idempotent no-op. A shrunk or rewritten file is
//! reconciled from the start instead of jumping to its end: the ids make that safe.
//!
//! Bindings that predate byte positions (they carry an entry-count cursor over an older
//! projection with positional ids) are migrated on their first sync:
//! - a session holding nothing but imported rows is upgraded: those positional rows are
//!   replaced, in one transaction, by turn-shaped rows read from the start of the transcript
//!   (this also restores the tool calls the old projection flattened into text);
//! - a session Jockey also wrote to keeps its rows, and transcript items no newer than its
//!   newest message are treated as already covered (they are either imported or Jockey's own
//!   turns, which the transcript also records), while newer ones are imported. This reconciles
//!   by time because the old count cannot be mapped to a byte position without re-running the
//!   old parsers; the newest row's timestamp is an upper bound of what the old cursor covered.

use super::tail::{self, Item};
use crate::db::app_session::{insert_import_rows, newest_message_at};
use crate::db::app_session_role::{set_transcript_position, ProviderConversation};
use crate::types::AppState;
use serde_json::Value;
use std::path::Path;

/// Entries written this close before a Jockey turn began, or after it ended, still count as
/// that turn's: the CLI flushes its transcript a moment after the result reaches Jockey.
const OWN_LEAD_MS: i64 = 500;
const OWN_TRAIL_MS: i64 = 3000;
/// Added to the legacy-migration boundary: the CLI timestamps its last lines a little after
/// Jockey stamps the turn's end.
const COVERED_SLACK_MS: i64 = 5000;

/// A span of wall-clock time during which a Jockey turn was driving the provider
/// conversation. Transcript items inside it were recorded by the engine already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnWindow {
    pub(crate) start: i64,
    pub(crate) end: Option<i64>,
}

impl OwnWindow {
    fn contains(&self, at: i64) -> bool {
        at >= self.start - OWN_LEAD_MS && self.end.is_none_or(|end| at <= end + OWN_TRAIL_MS)
    }
}

#[derive(Default)]
pub(crate) struct SyncCtx<'a> {
    pub(crate) own: &'a [OwnWindow],
    /// The file has been quiet long enough that a turn without an end marker counts as over.
    pub(crate) stale: bool,
}

impl SyncCtx<'_> {
    fn is_own(&self, at: i64) -> bool {
        self.own.iter().any(|w| w.contains(at))
    }
}

#[derive(Default)]
pub(crate) struct SyncResult {
    /// Messages newly stored, shaped as `message/added` carries them.
    pub(crate) added: Vec<Value>,
    /// The external turn in progress (id and message), held back from the database.
    pub(crate) open: Option<(String, Value)>,
    /// Whether the transcript says the provider is mid-turn; `None` when this read held no
    /// marker.
    pub(crate) running: Option<bool>,
    /// The position was invalid (file shrank or was rewritten) and the file was re-read whole.
    pub(crate) reconciled: bool,
}

enum Start {
    Resume(u64),
    Reconcile,
    First,
}

pub(crate) fn sync_binding(
    state: &AppState,
    session_id: &str,
    conversation: &ProviderConversation,
    path: &Path,
    ctx: &SyncCtx<'_>,
) -> Result<SyncResult, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("transcript unreadable: {e}"))?
        .len();
    let prefix = super::transcript_id_prefix(&conversation.runtime_kind, &conversation.provider_id);

    let start = match &conversation.position {
        Some(p)
            if p.offset <= len
                && tail::fingerprint(path, p.offset).is_ok_and(|f| f == p.fingerprint) =>
        {
            Start::Resume(p.offset)
        }
        Some(_) => Start::Reconcile,
        None => Start::First,
    };

    let mut boundary = None;
    let mut replace_legacy = false;
    let from = match start {
        Start::Resume(offset) => offset,
        Start::Reconcile => 0,
        Start::First => {
            let (all, imported) =
                crate::db::app_session::imported_message_counts(state, session_id, &prefix)?;
            if all > 0 && all == imported {
                replace_legacy = true;
            } else if all > 0 {
                boundary = Some(newest_message_at(state, session_id)? + COVERED_SLACK_MS);
            }
            0
        }
    };

    let Some(outcome) = tail::read(
        &conversation.runtime_kind,
        &conversation.role_name,
        path,
        from,
        ctx.stale,
    )
    .map_err(|e| format!("transcript read failed: {e}"))?
    else {
        return Ok(SyncResult::default());
    };

    let keep: Vec<&Item> = outcome
        .items
        .iter()
        .filter(|item| !ctx.is_own(item.at) && boundary.is_none_or(|b| item.at > b))
        .collect();
    let rows: Vec<_> = keep.iter().map(|item| item.to_row(&prefix)).collect();
    if replace_legacy && rows.is_empty() {
        // Nothing to upgrade to yet: keep the legacy rows and stay on the first-sync path,
        // so they are replaced once a whole turn exists instead of duplicated later.
        return Ok(SyncResult::default());
    }
    let inserted = insert_import_rows(
        state,
        session_id,
        &rows,
        replace_legacy.then_some(prefix.as_str()),
    )?;
    let added = keep
        .iter()
        .zip(inserted)
        .filter(|(_, new)| *new)
        .map(|(item, _)| item.to_json(&prefix))
        .collect();

    // Stored only after the rows are: a crash in between re-reads and dedupes by id.
    let fingerprint = tail::fingerprint(path, outcome.commit)
        .map_err(|e| format!("transcript fingerprint failed: {e}"))?;
    let position = crate::db::app_session_role::TranscriptPosition {
        offset: outcome.commit,
        fingerprint,
    };
    if conversation.position.as_ref() != Some(&position) {
        set_transcript_position(state, session_id, conversation, &position)?;
    }

    let open = outcome
        .open
        .filter(|item| !ctx.is_own(item.at))
        .map(|item| (format!("{prefix}:{}", item.id), item.to_json(&prefix)));
    Ok(SyncResult {
        added,
        open,
        running: outcome.running,
        reconciled: matches!(start, Start::Reconcile),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::app_session::{create_app_session_internal, save_app_message_internal};
    use crate::db::app_session_role::{list_provider_conversations, save_app_session_role_cli_id};
    use crate::db::{init_db, DbPool};
    use serde_json::json;
    use std::io::Write;
    use std::sync::Arc;

    pub(crate) fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(dir.path().join("t.sqlite3"), 2, "PRAGMA foreign_keys = ON;")
            .expect("pool");
        init_db(&pool.get().expect("conn")).expect("init");
        AppState {
            db: pool,
            role_cache: Arc::new(dashmap::DashMap::new()),
        }
    }

    const T0: i64 = 1_792_456_800_000; // 2026-10-20T00:40:00Z

    fn ts(offset_s: i64) -> String {
        // Whole-second offsets from T0, rendered the way Claude writes them.
        let total = T0 / 1000 + offset_s;
        let (h, m, s) = ((total % 86400) / 3600, (total % 3600) / 60, total % 60);
        format!("2026-10-20T{h:02}:{m:02}:{s:02}.000Z")
    }

    fn user(uuid: &str, text: &str, at: i64) -> Value {
        json!({"type":"user","uuid":uuid,"timestamp":ts(at),"message":{"role":"user","content":text}})
    }

    fn reply(uuid: &str, text: &str, at: i64) -> Value {
        json!({"type":"assistant","uuid":uuid,"timestamp":ts(at),
            "message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]}})
    }

    fn tool_step(uuid: &str, tool: &str, at: i64) -> Value {
        json!({"type":"assistant","uuid":uuid,"timestamp":ts(at),
            "message":{"role":"assistant","stop_reason":"tool_use","content":[
                {"type":"tool_use","id":tool,"name":"Bash","input":{"command":"ls"}}]}})
    }

    fn append(path: &Path, lines: &[Value]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        sid: String,
        path: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state = test_state(&dir);
            let session = create_app_session_internal(
                &state,
                Some("s"),
                None,
                Some("claude-code"),
                None,
                None,
            )
            .unwrap();
            save_app_session_role_cli_id(&state, &session.id, "claude-code", "Developer", "pid")
                .unwrap();
            let path = dir.path().join("pid.jsonl");
            Self {
                _dir: dir,
                state,
                sid: session.id,
                path,
            }
        }

        fn sync_with(&self, ctx: &SyncCtx<'_>) -> SyncResult {
            let conv = list_provider_conversations(&self.state, &self.sid)
                .unwrap()
                .pop()
                .unwrap();
            sync_binding(&self.state, &self.sid, &conv, &self.path, ctx).unwrap()
        }

        fn sync(&self) -> SyncResult {
            self.sync_with(&SyncCtx::default())
        }

        fn ids(&self) -> Vec<String> {
            crate::db::with_db(&self.state, |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT client_id FROM app_session_messages WHERE session_id = ?1 ORDER BY id",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([&self.sid], |r| r.get::<_, Option<String>>(0))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok().flatten())
                    .collect();
                Ok(rows)
            })
            .unwrap()
        }
    }

    fn texts(result: &SyncResult) -> Vec<String> {
        result
            .added
            .iter()
            .map(|m| m["text"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn syncs_only_appended_turns_and_is_idempotent() {
        let f = Fixture::new();
        append(
            &f.path,
            &[user("u1", "hello", 0), reply("a1", "hi there", 1)],
        );
        assert_eq!(texts(&f.sync()), ["hello", "hi there"]);
        assert!(f.sync().added.is_empty());
        assert_eq!(
            f.ids(),
            ["import:claude-code:pid:u:u1", "import:claude-code:pid:t:a1"]
        );

        append(&f.path, &[user("u2", "again", 10), reply("a2", "sure", 11)]);
        assert_eq!(texts(&f.sync()), ["again", "sure"]);
        assert!(f.sync().added.is_empty());
    }

    #[test]
    fn a_partial_trailing_line_waits_for_its_newline() {
        let f = Fixture::new();
        append(&f.path, &[user("u1", "hello", 0)]);
        let half = reply("a1", "hi there", 1).to_string();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&f.path)
            .unwrap();
        write!(file, "{}", &half[..half.len() / 2]).unwrap();
        assert_eq!(texts(&f.sync()), ["hello"]);
        writeln!(file, "{}", &half[half.len() / 2..]).unwrap();
        assert_eq!(texts(&f.sync()), ["hi there"]);
    }

    #[test]
    fn an_open_turn_is_held_then_stored_whole_when_it_ends() {
        let f = Fixture::new();
        append(
            &f.path,
            &[user("u1", "build", 0), tool_step("a1", "tu1", 1)],
        );
        let first = f.sync();
        assert_eq!(texts(&first), ["build"]);
        let (id, live) = first.open.expect("live preview");
        assert_eq!(id, "import:claude-code:pid:t:a1");
        assert_eq!(live["segments"][0]["kind"], "tool");
        assert_eq!(first.running, Some(true));
        assert_eq!(f.ids().len(), 1, "nothing of the turn is persisted yet");

        // Still open, more content: re-read from the turn start, preview grows.
        append(
            &f.path,
            &[
                json!({"type":"user","uuid":"r1","timestamp":ts(2),"message":{"role":"user",
                "content":[{"type":"tool_result","tool_use_id":"tu1","content":"ok"}]}}),
            ],
        );
        let second = f.sync();
        assert!(second.added.is_empty());
        assert_eq!(
            second.open.unwrap().1["toolCalls"][0]["status"],
            "completed"
        );

        append(&f.path, &[reply("a2", "built", 3)]);
        let done = f.sync();
        assert!(done.open.is_none());
        assert_eq!(done.running, Some(false));
        assert_eq!(done.added.len(), 1);
        let msg = &done.added[0];
        assert_eq!(msg["id"], "import:claude-code:pid:t:a1");
        assert_eq!(msg["text"], "built");
        assert_eq!(msg["toolCalls"][0]["rawOutput"], "ok");
        assert_eq!(f.ids().len(), 2);
    }

    #[test]
    fn a_shrunk_or_rewritten_file_reconciles_without_duplicating() {
        let f = Fixture::new();
        append(
            &f.path,
            &[
                user("u1", "one", 0),
                reply("a1", "first", 1),
                user("u2", "two", 5),
                reply("a2", "second", 6),
            ],
        );
        f.sync();
        assert_eq!(f.ids().len(), 4);

        // Rewritten shorter (compaction): the stored position is no longer valid.
        std::fs::write(&f.path, "").unwrap();
        append(
            &f.path,
            &[
                user("u1", "one", 0),
                reply("a1", "first", 1),
                user("u3", "three", 20),
                reply("a3", "third", 21),
            ],
        );
        let result = f.sync();
        assert!(result.reconciled);
        assert_eq!(
            texts(&result),
            ["three", "third"],
            "known ids are not re-added"
        );
        assert_eq!(f.ids().len(), 6);
        assert!(!f.sync().reconciled);

        // Same length, different bytes before the offset.
        let text = std::fs::read_to_string(&f.path)
            .unwrap()
            .replace("third", "THIRD");
        std::fs::write(&f.path, text).unwrap();
        assert!(f.sync().reconciled);
        assert_eq!(f.ids().len(), 6);
    }

    #[test]
    fn jockeys_own_turn_is_skipped_by_time_not_by_jumping_to_the_end() {
        let f = Fixture::new();
        append(&f.path, &[user("u1", "hello", 0), reply("a1", "hi", 1)]);
        f.sync();
        // Jockey ran a turn at +100..+110 s, and a terminal ran one at +200 s.
        append(
            &f.path,
            &[
                user("own-u", "from jockey", 100),
                reply("own-a", "jockey reply", 105),
            ],
        );
        let own = [OwnWindow {
            start: T0 + 99_800,
            end: Some(T0 + 110_000),
        }];
        let during = f.sync_with(&SyncCtx {
            own: &own,
            stale: false,
        });
        assert!(during.added.is_empty());
        append(
            &f.path,
            &[
                user("t-u", "from terminal", 200),
                reply("t-a", "terminal reply", 201),
            ],
        );
        let after = f.sync_with(&SyncCtx {
            own: &own,
            stale: false,
        });
        assert_eq!(texts(&after), ["from terminal", "terminal reply"]);
    }

    #[test]
    fn legacy_pure_imports_are_upgraded_in_place() {
        let f = Fixture::new();
        append(&f.path, &[user("u1", "hello", 0), reply("a1", "hi", 1)]);
        // What the old importer left: positional ids, a count cursor, no byte position.
        crate::db::app_session::insert_import_rows(
            &f.state,
            &f.sid,
            &[
                crate::db::app_session::ImportRow {
                    client_id: "import:claude-code:pid:0".into(),
                    role_name: "user".into(),
                    content: "hello".into(),
                    content_type: None,
                    payload: None,
                    created_at: 5,
                },
                crate::db::app_session::ImportRow {
                    client_id: "import:claude-code:pid:1".into(),
                    role_name: "Developer".into(),
                    content: "hi".into(),
                    content_type: None,
                    payload: None,
                    created_at: 6,
                },
            ],
            None,
        )
        .unwrap();
        let result = f.sync();
        assert_eq!(result.added.len(), 2);
        assert_eq!(
            f.ids(),
            ["import:claude-code:pid:u:u1", "import:claude-code:pid:t:a1"]
        );
    }

    #[test]
    fn legacy_sessions_jockey_wrote_to_import_only_newer_transcript_items() {
        let f = Fixture::new();
        // Imported long ago, then Jockey ran a turn at +100 s (its rows are newest).
        append(
            &f.path,
            &[
                user("u1", "old", 0),
                reply("a1", "old reply", 1),
                user("j-u", "jockey", 100),
                reply("j-a", "jockey reply", 101),
            ],
        );
        save_app_message_internal(
            &f.state,
            &f.sid,
            Some("import:claude-code:pid:0"),
            "user",
            "old",
            None,
            None,
        )
        .unwrap();
        save_app_message_internal(
            &f.state,
            &f.sid,
            Some("jockey-user"),
            "user",
            "jockey",
            None,
            None,
        )
        .unwrap();
        let newest = crate::db::app_session::newest_message_at(&f.state, &f.sid).unwrap();
        // Make the Jockey rows newer than the transcript entries they correspond to.
        assert!(newest > 0);
        crate::db::with_db(&f.state, |conn| {
            conn.execute(
                "UPDATE app_session_messages SET created_at = ?1",
                [T0 + 110_000],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE app_sessions SET last_active_at = ?1",
                [T0 + 110_000],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        append(
            &f.path,
            &[user("t-u", "later", 300), reply("t-a", "later reply", 301)],
        );
        let result = f.sync();
        assert_eq!(texts(&result), ["later", "later reply"]);
        assert!(f.sync().added.is_empty());
    }

    #[test]
    fn stale_open_turns_are_stored_as_finished() {
        let f = Fixture::new();
        append(&f.path, &[user("u1", "go", 0), tool_step("a1", "tu1", 1)]);
        let result = f.sync_with(&SyncCtx {
            own: &[],
            stale: true,
        });
        assert!(result.open.is_none());
        assert_eq!(result.added.len(), 2);
        assert_eq!(result.added[1]["toolCalls"][0]["status"], "interrupted");
    }
}
