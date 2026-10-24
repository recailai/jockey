//! ACP `session/list` for the import dialog. A short-lived adapter process is started, asked
//! for its conversations, and stopped; it never opens a session, so nothing is resumed or
//! replayed. Only adapters that are already installed are used.

use crate::acp::protocol as acp;
use crate::runtime_kind::RuntimeKind;

use super::probe::probe_runtime;

/// Pages are adapter-sized (about 50); this bounds a runaway cursor, not a real history.
const MAX_PAGES: usize = 200;

#[derive(Debug, Clone)]
pub(crate) struct ListedSession {
    pub(crate) session_id: String,
    pub(crate) cwd: String,
    pub(crate) title: Option<String>,
    pub(crate) updated_at_ms: Option<i64>,
}

/// Lists a runtime's conversations, or `Ok(None)` when its adapter is not installed or does not
/// advertise `sessionCapabilities.list`.
pub(crate) fn list_runtime_sessions(
    kind: RuntimeKind,
) -> Result<Option<Vec<ListedSession>>, String> {
    probe_runtime(kind, "session/list", |conn, init| async move {
        if init.agent_capabilities.session_capabilities.list.is_none() {
            return Ok(None);
        }
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page: acp::ListSessionsResponse = conn
                .request(
                    "session/list",
                    acp::ListSessionsRequest::new().cursor(cursor.clone()),
                )
                .await
                .map_err(|e| format!("session/list: {e}"))?;
            out.extend(page.sessions.into_iter().map(|s| ListedSession {
                session_id: s.session_id.to_string(),
                cwd: s.cwd.to_string_lossy().into_owned(),
                title: s.title.filter(|t| !t.trim().is_empty()),
                updated_at_ms: s.updated_at.as_deref().and_then(parse_rfc3339_ms),
            }));
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
        Ok(Some(out))
    })
}

/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` → epoch milliseconds. Adapters send ISO 8601 for
/// `updatedAt`; a small parser avoids a date crate for one field.
fn parse_rfc3339_ms(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = raw.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (clock, offset_ms) = if let Some(c) = rest.strip_suffix(['Z', 'z']) {
        (c, 0)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (c, off) = rest.split_at(at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':')?;
        (c, sign * (num(oh)? * 60 + num(om)?) * 60_000)
    };
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, ""));
    let mut t = hms.splitn(3, ':');
    let (h, mi, sec) = (num(t.next()?)?, num(t.next()?)?, num(t.next()?)?);
    let millis = format!("{frac:0<3}").get(..3).and_then(num).unwrap_or(0);
    // Days from 1970-01-01 to the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + millis - offset_ms)
}

#[cfg(test)]
mod tests {
    use super::parse_rfc3339_ms;

    #[test]
    fn parses_adapter_timestamps() {
        assert_eq!(
            parse_rfc3339_ms("2026-10-24T03:21:47.000Z"),
            Some(1_792_812_107_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-24T11:21:47+08:00"),
            Some(1_792_812_107_000)
        );
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("yesterday"), None);
    }
}
