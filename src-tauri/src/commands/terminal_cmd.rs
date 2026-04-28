use dashmap::DashMap;
use serde::Serialize;
use std::os::fd::FromRawFd;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::db::get_state;
use crate::types::AppState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalStartResponse {
    pub terminal_id: String,
    pub cwd: String,
    pub shell: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TerminalOutputEvent {
    terminal_id: String,
    app_session_id: String,
    data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TerminalExitEvent {
    terminal_id: String,
    app_session_id: String,
    exit_code: Option<i32>,
}

struct TerminalHandle {
    writer: Arc<Mutex<tokio::fs::File>>,
    pid: Option<u32>,
}

static TERMINALS: OnceLock<DashMap<String, TerminalHandle>> = OnceLock::new();

fn terminals() -> &'static DashMap<String, TerminalHandle> {
    TERMINALS.get_or_init(DashMap::new)
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

#[cfg(unix)]
fn open_pty() -> Result<
    (
        tokio::fs::File,
        tokio::fs::File,
        std::process::Stdio,
        std::process::Stdio,
        std::process::Stdio,
    ),
    String,
> {
    let mut master_fd = 0;
    let mut slave_fd = 0;
    let mut winsize = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let master_writer = master.try_clone().map_err(|e| e.to_string())?;
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };
    let slave_out = slave.try_clone().map_err(|e| e.to_string())?;
    let slave_err = slave.try_clone().map_err(|e| e.to_string())?;

    Ok((
        tokio::fs::File::from_std(master),
        tokio::fs::File::from_std(master_writer),
        std::process::Stdio::from(slave),
        std::process::Stdio::from(slave_out),
        std::process::Stdio::from(slave_err),
    ))
}

async fn forward_output<R>(
    app: AppHandle,
    terminal_id: String,
    app_session_id: String,
    mut reader: R,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = [0_u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let data = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = app.emit(
                    "terminal/session_output",
                    TerminalOutputEvent {
                        terminal_id: terminal_id.clone(),
                        app_session_id: app_session_id.clone(),
                        data,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "terminal/session_output",
                    TerminalOutputEvent {
                        terminal_id,
                        app_session_id,
                        data: format!("\r\nterminal read error: {e}\r\n"),
                    },
                );
                break;
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn start_terminal_session(
    app: AppHandle,
    state: State<'_, AppState>,
    app_session_id: String,
) -> Result<TerminalStartResponse, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }

    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), sid)
        .unwrap_or_else(crate::resolve_chat_cwd);
    let shell = default_shell();
    let terminal_id = uuid::Uuid::new_v4().to_string();
    let (reader, writer, stdin, stdout, stderr) = open_pty()?;

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg("-l")
        .current_dir(&cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true)
        .process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn terminal failed: {e}"))?;
    let pid = child.id();

    terminals().insert(
        terminal_id.clone(),
        TerminalHandle {
            writer: Arc::new(Mutex::new(writer)),
            pid,
        },
    );

    tauri::async_runtime::spawn(forward_output(
        app.clone(),
        terminal_id.clone(),
        sid.to_string(),
        reader,
    ));

    let exit_terminal_id = terminal_id.clone();
    let exit_sid = sid.to_string();
    tauri::async_runtime::spawn(async move {
        let status = child.wait().await.ok();
        terminals().remove(&exit_terminal_id);
        let _ = app.emit(
            "terminal/session_exit",
            TerminalExitEvent {
                terminal_id: exit_terminal_id,
                app_session_id: exit_sid,
                exit_code: status.and_then(|s| s.code()),
            },
        );
    });

    Ok(TerminalStartResponse {
        terminal_id,
        cwd,
        shell,
    })
}

#[tauri::command]
pub(crate) async fn write_terminal_session(
    terminal_id: String,
    data: String,
) -> Result<(), String> {
    let writer = terminals()
        .get(&terminal_id)
        .map(|h| h.writer.clone())
        .ok_or_else(|| "terminal not found".to_string())?;
    let mut guard = writer.lock().await;
    guard
        .write_all(data.as_bytes())
        .await
        .map_err(|e| format!("terminal write failed: {e}"))?;
    guard
        .flush()
        .await
        .map_err(|e| format!("terminal flush failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn stop_terminal_session(terminal_id: String) -> Result<(), String> {
    let Some((_id, handle)) = terminals().remove(&terminal_id) else {
        return Ok(());
    };
    if let Some(pid) = handle.pid {
        unsafe {
            let _ = libc::kill(-(pid as i32), libc::SIGTERM);
            let _ = libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    Ok(())
}
