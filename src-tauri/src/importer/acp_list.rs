//! ACP `session/list` as a second discovery source next to the transcript scanners.
//!
//! The file scanners stay authoritative for which Claude, Codex and Pi conversations exist:
//! they filter probe, sub-agent and empty transcripts, and their files carry the history that
//! import copies. ACP contributes what files cannot: Claude's generated session summaries as
//! titles, and the whole list for runtimes Jockey has no transcript reader for (Antigravity). Listing starts each installed adapter once; missing adapters are skipped.

use super::DiscoveredSession;
use crate::acp::session::list::{list_runtime_sessions, ListedSession};
use crate::runtime_kind::RuntimeKind;
use std::collections::HashMap;
use std::path::PathBuf;

const FILE_BACKED: [RuntimeKind; 3] = [
    RuntimeKind::ClaudeCode,
    RuntimeKind::CodexCli,
    RuntimeKind::PiCli,
];
const ACP_ONLY: [(RuntimeKind, &str); 1] = [(RuntimeKind::AntigravityCli, "Antigravity")];
/// Runtimes whose listed title is a real summary. Codex and Pi list the raw first message
/// (attachment headers included), which reads worse than the scanner's cleaned title.
const SUMMARY_TITLES: [RuntimeKind; 1] = [RuntimeKind::ClaudeCode];

/// Every installed adapter's list, fetched in parallel. Failures are logged by the lister and
/// leave that runtime out; the import dialog still shows what the files have.
fn list_all() -> HashMap<RuntimeKind, Vec<ListedSession>> {
    let kinds = FILE_BACKED
        .iter()
        .copied()
        .chain(ACP_ONLY.iter().map(|(k, _)| *k))
        .collect::<Vec<_>>();
    std::thread::scope(|scope| {
        let handles = kinds
            .iter()
            .map(|&kind| (kind, scope.spawn(move || list_runtime_sessions(kind))))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .filter_map(|(kind, h)| match h.join() {
                Ok(Ok(Some(sessions))) => Some((kind, sessions)),
                _ => None,
            })
            .collect()
    })
}

/// Adds ACP's view to scanned sessions: agent titles for known conversations and the
/// conversations of ACP-only runtimes. `belongs` filters ACP-only entries to the scope.
pub(super) fn enrich(sessions: &mut Vec<DiscoveredSession>, belongs: impl Fn(&str) -> bool) {
    let listed = list_all();
    merge(sessions, &listed, belongs);
}

fn merge(
    sessions: &mut Vec<DiscoveredSession>,
    listed: &HashMap<RuntimeKind, Vec<ListedSession>>,
    belongs: impl Fn(&str) -> bool,
) {
    let titles: HashMap<(&str, &str), &str> = listed
        .iter()
        .filter(|(kind, _)| SUMMARY_TITLES.contains(kind))
        .flat_map(|(kind, list)| {
            list.iter().filter_map(move |s| {
                Some((
                    (kind.runtime_key(), s.session_id.as_str()),
                    s.title.as_deref()?,
                ))
            })
        })
        .collect();
    for session in sessions.iter_mut() {
        if let Some(title) = titles.get(&(session.runtime_kind, session.session_id.as_str())) {
            // The fork marker is Jockey's; the agent's title does not carry it.
            session.title = match session.title.strip_prefix("[Fork] ") {
                Some(_) if !title.starts_with("[Fork]") => format!("[Fork] {title}"),
                _ => title.to_string(),
            };
        }
    }
    for (kind, agent) in ACP_ONLY {
        for s in listed.get(&kind).into_iter().flatten() {
            if !belongs(&s.cwd) {
                continue;
            }
            let at = s.updated_at_ms.unwrap_or(0);
            sessions.push(DiscoveredSession {
                runtime_kind: kind.runtime_key(),
                agent,
                session_id: s.session_id.clone(),
                file_path: PathBuf::new(),
                cwd: s.cwd.clone(),
                title: s.title.clone().unwrap_or_else(|| {
                    format!("{agent}_{}", &s.session_id[..s.session_id.len().min(8)])
                }),
                created_at: at,
                last_active_at: at,
                turn_count: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(kind: &'static str, id: &str, title: &str) -> DiscoveredSession {
        DiscoveredSession {
            runtime_kind: kind,
            agent: "Codex",
            session_id: id.into(),
            file_path: PathBuf::from("/x.jsonl"),
            cwd: "/w/app".into(),
            title: title.into(),
            created_at: 1,
            last_active_at: 1,
            turn_count: 3,
        }
    }

    fn listed(id: &str, cwd: &str, title: Option<&str>) -> ListedSession {
        ListedSession {
            session_id: id.into(),
            cwd: cwd.into(),
            title: title.map(Into::into),
            updated_at_ms: Some(5),
        }
    }

    #[test]
    fn acp_titles_win_and_acp_only_runtimes_are_added_in_scope() {
        let mut sessions = vec![
            found("claude-code", "a", "first message"),
            found("claude-code", "f", "[Fork] first message"),
            found("codex-cli", "c", "cleaned title"),
        ];
        let listed = HashMap::from([
            (
                RuntimeKind::ClaudeCode,
                vec![
                    listed("a", "/w/app", Some("Fix login")),
                    listed("f", "/w/app", Some("Fix login")),
                    // Known to ACP but not to the scanner (e.g. a probe it filtered): not added.
                    listed("probe", "/w/app", Some("Reply only with OK")),
                ],
            ),
            (
                RuntimeKind::CodexCli,
                vec![listed(
                    "c",
                    "/w/app",
                    Some("# Files mentioned by the user: raw"),
                )],
            ),
            (
                RuntimeKind::AntigravityCli,
                vec![
                    listed("g1", "/w/app/sub", Some("Plan")),
                    listed("g2", "/elsewhere", None),
                ],
            ),
        ]);
        merge(&mut sessions, &listed, |cwd| cwd.starts_with("/w/app"));
        let titles: Vec<_> = sessions
            .iter()
            .map(|s| (s.session_id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(
            titles,
            [
                ("a", "Fix login"),
                ("f", "[Fork] Fix login"),
                ("c", "cleaned title"),
                ("g1", "Plan")
            ]
        );
        let agy = sessions.iter().find(|s| s.session_id == "g1").unwrap();
        assert_eq!(agy.runtime_kind, "antigravity-cli");
        assert!(agy.file_path.as_os_str().is_empty());
        assert_eq!(agy.last_active_at, 5);
    }
}
