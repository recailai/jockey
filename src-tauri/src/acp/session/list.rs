//! ACP `session/list` for the import dialog. A short-lived adapter process is started, asked
//! for its conversations, and stopped; it never opens a session, so nothing is resumed or
//! replayed. Only adapters that are already installed are used.

use crate::acp::protocol as acp;
use crate::runtime_kind::RuntimeKind;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::adapter::{acp_log, installed_stdio_adapter};
use super::super::client::JockeyUiClient;
use super::super::transport::{run_io, AcpConnection};

/// Whole listing for one runtime, spawn included. The dialog waits on the slowest runtime, so a
/// hung adapter costs this much once and is then skipped.
const LIST_TIMEOUT: Duration = Duration::from_secs(12);
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

/// Runs `op` against a short-lived, already-installed adapter that has completed `initialize`;
/// `Ok(None)` when the adapter is not installed or `op` reports the capability missing. No
/// session is ever opened. Blocking: runs on its own thread and runtime so it can be called from
/// any context, including a Tokio worker.
pub(crate) fn probe_runtime<T, F, Fut>(
    kind: RuntimeKind,
    op_name: &'static str,
    op: F,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce(Rc<AcpConnection>, acp::InitializeResponse) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Option<T>, String>> + 'static,
{
    let Some(spec) = installed_stdio_adapter(kind) else {
        return Ok(None);
    };
    std::thread::Builder::new()
        .name(format!("acp-probe-{}", kind.runtime_key()))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                tokio::time::timeout(
                    LIST_TIMEOUT,
                    probe_with(
                        spec.runtime_key,
                        op_name,
                        &spec.binary,
                        &spec.args,
                        &spec.env,
                        op,
                    ),
                )
                .await
                .map_err(|_| format!("{}: {op_name} timed out", spec.runtime_key))?
            })
        })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| format!("{op_name} thread panicked"))?
}

async fn probe_with<T, F, Fut>(
    runtime_key: &'static str,
    op_name: &'static str,
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    op: F,
) -> Result<Option<T>, String>
where
    F: FnOnce(Rc<AcpConnection>, acp::InitializeResponse) -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, String>>,
{
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{runtime_key}: {e}"))?;
    let stdin = child.stdin.take().ok_or("stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;

    // No session is ever opened, so the client only has to exist for the IO loop; it ignores
    // updates (no expected session) and is never asked for files or terminals.
    let client = Rc::new(JockeyUiClient {
        delta_slot: Arc::new(Mutex::new(None)),
        auto_approve: false,
        runtime_key: runtime_key.to_string(),
        role_name: String::new(),
        app_session_id: String::new(),
        expected_session_id: Arc::new(Mutex::new(None)),
        mode_state: Rc::new(RefCell::new(None)),
        config_state: Rc::new(RefCell::new(Vec::new())),
        terminals: Rc::new(RefCell::new(std::collections::HashMap::new())),
        subagents: Default::default(),
    });
    let conn = Rc::new(AcpConnection::new(stdin));
    let io = tokio::task::spawn_local(run_io(stdout, conn.clone(), client));

    let result = async {
        let init = conn
            .initialize(
                acp::InitializeRequest::new(acp::ProtocolVersion::V1)
                    .client_info(acp::Implementation::new("jockey", "0.1.0").title("Jockey")),
            )
            .await
            .map_err(|e| format!("{runtime_key}: initialize: {e}"))?;
        op(conn.clone(), init)
            .await
            .map_err(|e| format!("{runtime_key}: {e}"))
    }
    .await;

    io.abort();
    let _ = child.kill().await;
    acp_log(
        "probe.done",
        json!({
            "runtime": runtime_key,
            "op": op_name,
            "found": result.as_ref().ok().map(Option::is_some),
            "error": result.as_ref().err(),
        }),
    );
    result
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
