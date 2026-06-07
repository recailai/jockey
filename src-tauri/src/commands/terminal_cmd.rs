use dashmap::DashMap;
use serde::Serialize;
use std::os::fd::FromRawFd;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as AsyncMutex;

use crate::db::get_state;
use crate::types::AppState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalStartResponse {
    pub terminal_id: String,
    pub cwd: String,
    pub shell: String,
    pub reused: bool,
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
    writer: Arc<AsyncMutex<tokio::fs::File>>,
    master: Arc<Mutex<std::fs::File>>,
    pid: Option<u32>,
    app_session_id: String,
    cwd: String,
    shell: String,
}

static TERMINALS: OnceLock<DashMap<String, TerminalHandle>> = OnceLock::new();
static SESSION_TERMINALS: OnceLock<DashMap<String, String>> = OnceLock::new();

fn terminals() -> &'static DashMap<String, TerminalHandle> {
    TERMINALS.get_or_init(DashMap::new)
}

fn session_terminals() -> &'static DashMap<String, String> {
    SESSION_TERMINALS.get_or_init(DashMap::new)
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

#[cfg(unix)]
fn set_winsize(master: &std::fs::File, rows: u16, cols: u16) -> Result<(), String> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_winsize(_master: &std::fs::File, _rows: u16, _cols: u16) -> Result<(), String> {
    Err("terminal resize is only supported on unix".to_string())
}

#[cfg(unix)]
fn open_pty() -> Result<
    (
        tokio::fs::File,
        tokio::fs::File,
        Arc<Mutex<std::fs::File>>,
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
    let master_ioctl = Arc::new(Mutex::new(
        master
            .try_clone()
            .map_err(|e| format!("clone master pty failed: {e}"))?,
    ));
    let master_reader = master
        .try_clone()
        .map_err(|e| format!("clone master reader failed: {e}"))?;
    let master_writer = master
        .try_clone()
        .map_err(|e| format!("clone master writer failed: {e}"))?;
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };
    let slave_out = slave.try_clone().map_err(|e| e.to_string())?;
    let slave_err = slave.try_clone().map_err(|e| e.to_string())?;

    Ok((
        tokio::fs::File::from_std(master_reader),
        tokio::fs::File::from_std(master_writer),
        master_ioctl,
        std::process::Stdio::from(slave),
        std::process::Stdio::from(slave_out),
        std::process::Stdio::from(slave_err),
    ))
}

#[cfg(not(unix))]
fn open_pty() -> Result<
    (
        tokio::fs::File,
        tokio::fs::File,
        Arc<Mutex<std::fs::File>>,
        std::process::Stdio,
        std::process::Stdio,
        std::process::Stdio,
    ),
    String,
> {
    Err("PTY is only supported on unix".to_string())
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

fn kill_handle(handle: &TerminalHandle) {
    if let Some(pid) = handle.pid {
        unsafe {
            let _ = libc::kill(-(pid as i32), libc::SIGTERM);
            let _ = libc::kill(pid as i32, libc::SIGTERM);
        }
    }
}

fn remove_terminal(terminal_id: &str) {
    if let Some((_, handle)) = terminals().remove(terminal_id) {
        session_terminals().remove(&handle.app_session_id);
        kill_handle(&handle);
    }
}

#[tauri::command]
pub(crate) async fn start_terminal_session(
    app: AppHandle,
    state: State<'_, AppState>,
    app_session_id: String,
    force_new: Option<bool>,
) -> Result<TerminalStartResponse, String> {
    let sid = app_session_id.trim();
    if sid.is_empty() {
        return Err("app session id required".to_string());
    }

    if force_new.unwrap_or(false) {
        if let Some(entry) = session_terminals().get(sid) {
            remove_terminal(entry.value());
        }
    } else if let Some(entry) = session_terminals().get(sid) {
        let terminal_id = entry.value().clone();
        if let Some(handle) = terminals().get(&terminal_id) {
            return Ok(TerminalStartResponse {
                terminal_id,
                cwd: handle.cwd.clone(),
                shell: handle.shell.clone(),
                reused: true,
            });
        }
        session_terminals().remove(sid);
    }

    let cwd = crate::db::app_session::get_app_session_cwd(get_state(&state), sid)
        .unwrap_or_else(crate::resolve_chat_cwd);
    let shell = default_shell();
    let terminal_id = uuid::Uuid::new_v4().to_string();
    let (reader, writer, master, stdin, stdout, stderr) = open_pty()?;

    {
        let guard = master
            .lock()
            .map_err(|e| format!("lock master pty failed: {e}"))?;
        set_winsize(&guard, 24, 80)?;
    }

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg("-i")
        .current_dir(&cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true);

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn terminal failed: {e}"))?;
    let pid = child.id();

    terminals().insert(
        terminal_id.clone(),
        TerminalHandle {
            writer: Arc::new(AsyncMutex::new(writer)),
            master,
            pid,
            app_session_id: sid.to_string(),
            cwd: cwd.clone(),
            shell: shell.clone(),
        },
    );
    session_terminals().insert(sid.to_string(), terminal_id.clone());

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
        remove_terminal(&exit_terminal_id);
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
        reused: false,
    })
}

#[tauri::command]
pub(crate) async fn resize_terminal_session(
    terminal_id: String,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    if rows == 0 || cols == 0 {
        return Ok(());
    }
    let master = terminals()
        .get(&terminal_id)
        .map(|h| h.master.clone())
        .ok_or_else(|| "terminal not found".to_string())?;
    let guard = master
        .lock()
        .map_err(|e| format!("lock master pty failed: {e}"))?;
    set_winsize(&guard, rows, cols)
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
    remove_terminal(&terminal_id);
    Ok(())
}
