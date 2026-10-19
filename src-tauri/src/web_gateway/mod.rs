mod api;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Listener};

const DEFAULT_PORT: u16 = 8420;
const TOKEN_FILE: &str = "companion_token";
/// Upper bound on how long stop waits for in-flight requests (e.g. a long
/// `assistant_chat`) before the server future is dropped outright.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
const FORWARDED_EVENTS: &[&str] = &[
    "acp/delta",
    "acp/stream",
    crate::engine::turns::TURN_STARTED_EVENT,
    crate::engine::turns::TURN_FINISHED_EVENT,
    crate::engine::queue::QUEUE_CHANGED_EVENT,
    crate::engine::service::MESSAGE_ADDED_EVENT,
    api::CHANGED_EVENT,
];

#[derive(Clone, Debug)]
pub struct GatewayEvent {
    pub event_name: String,
    pub payload: String,
}

#[derive(Clone)]
struct ServerContext {
    app: AppHandle,
    // A watch rather than a fixed string so regenerating the token while the
    // server runs revokes old links immediately and ends their SSE streams.
    token: tokio::sync::watch::Receiver<String>,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
    shutdown: tokio::sync::watch::Receiver<bool>,
}

struct RunningServer {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    token_tx: tokio::sync::watch::Sender<String>,
    task: tokio::task::JoinHandle<()>,
}

pub struct GatewayManager {
    port: u16,
    token: Option<String>,
    server: Option<RunningServer>,
    broadcaster: tokio::sync::broadcast::Sender<GatewayEvent>,
    // Tauri listeners are app-global and outlive a server run; installing them
    // per start would duplicate every forwarded event after a restart.
    listeners_installed: bool,
}

impl GatewayManager {
    fn is_running(&self) -> bool {
        self.server.as_ref().is_some_and(|s| !s.task.is_finished())
    }

    fn token(&mut self) -> String {
        if self.token.is_none() {
            self.token = Some(load_or_create_token());
        }
        self.token.clone().unwrap_or_default()
    }

    fn set_token(&mut self, token: String) {
        persist_token(&token);
        if let Some(server) = &self.server {
            let _ = server.token_tx.send(token.clone());
        }
        self.token = Some(token);
    }

    fn status(&mut self) -> GatewayStatus {
        let running = self.is_running();
        let token = self.token();
        build_status(running, self.port, &token)
    }
}

static MANAGER: OnceLock<Mutex<GatewayManager>> = OnceLock::new();

fn get_manager() -> &'static Mutex<GatewayManager> {
    MANAGER.get_or_init(|| {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Mutex::new(GatewayManager {
            port: DEFAULT_PORT,
            token: None,
            server: None,
            broadcaster: tx,
            listeners_installed: false,
        })
    })
}

fn token_path() -> Option<PathBuf> {
    crate::acp::app_data_dir().map(|dir| dir.join(TOKEN_FILE))
}

fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The token is persisted so links saved on a phone keep working across app
/// restarts; it is only rotated by an explicit regenerate.
fn load_or_create_token() -> String {
    if let Some(existing) = token_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return existing;
    }
    let token = new_token();
    persist_token(&token);
    token
}

fn persist_token(token: &str) {
    let Some(path) = token_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, token) {
        eprintln!("[web_gateway] failed to persist token: {e}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub local_url: String,
    pub tailscale_url: Option<String>,
    pub lan_urls: Vec<String>,
}

/// Detect local network IPs and Tailscale IP
pub fn detect_network_ips() -> (Option<String>, Vec<String>) {
    let mut tailscale_ip = None;
    let mut lan_ips = Vec::new();

    // 1. Try running tailscale CLI
    let ts_candidates = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/opt/homebrew/bin/tailscale",
        "/usr/local/bin/tailscale",
    ];
    for bin in ts_candidates {
        if let Ok(output) = std::process::Command::new(bin).arg("ip").arg("-4").output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !s.is_empty() && s.starts_with("100.") {
                    tailscale_ip = Some(s);
                    break;
                }
            }
        }
    }

    // 2. Try ifconfig to find other IPs
    if let Ok(output) = std::process::Command::new("ifconfig").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("inet ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let ip = parts[1];
                    if ip == "127.0.0.1" {
                        continue;
                    }
                    if ip.starts_with("100.") && tailscale_ip.is_none() {
                        tailscale_ip = Some(ip.to_string());
                    } else if is_private_lan_ip(ip) && !lan_ips.iter().any(|l| l == ip) {
                        lan_ips.push(ip.to_string());
                    }
                }
            }
        }
    }

    (tailscale_ip, lan_ips)
}

/// RFC 1918 ranges only; a bare `172.` prefix would also match public
/// addresses outside 172.16.0.0/12.
fn is_private_lan_ip(ip: &str) -> bool {
    ip.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|addr| addr.is_private())
}

fn build_status(running: bool, port: u16, token: &str) -> GatewayStatus {
    let (tailscale_ip, lan_ips) = detect_network_ips();
    let local_url = format!("http://localhost:{port}/?token={token}");
    let tailscale_url = tailscale_ip.map(|ip| format!("http://{ip}:{port}/?token={token}"));
    let lan_urls = lan_ips
        .into_iter()
        .map(|ip| format!("http://{ip}:{port}/?token={token}"))
        .collect();

    GatewayStatus {
        running,
        port,
        token: token.to_string(),
        local_url,
        tailscale_url,
        lan_urls,
    }
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_gateway_status_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    Ok(mgr.status())
}

#[tauri::command]
pub fn regenerate_gateway_token_cmd() -> Result<GatewayStatus, String> {
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    mgr.set_token(new_token());
    Ok(mgr.status())
}

fn install_event_forwarders(app: &AppHandle, mgr: &mut GatewayManager) {
    if mgr.listeners_installed {
        return;
    }
    for &name in FORWARDED_EVENTS {
        let tx = mgr.broadcaster.clone();
        app.listen_any(name, move |ev| {
            let _ = tx.send(GatewayEvent {
                event_name: name.to_string(),
                payload: ev.payload().to_string(),
            });
        });
    }
    mgr.listeners_installed = true;
}

#[tauri::command]
pub async fn start_gateway_cmd(
    app: AppHandle,
    port: Option<u16>,
    token: Option<String>,
) -> Result<GatewayStatus, String> {
    let (assigned_port, assigned_token, broadcaster) = {
        let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
        if mgr.is_running() {
            return Ok(mgr.status());
        }
        if let Some(p) = port {
            mgr.port = p;
        }
        if let Some(t) = token.filter(|t| !t.trim().is_empty()) {
            mgr.set_token(t.trim().to_string());
        }
        install_event_forwarders(&app, &mut mgr);
        (mgr.port, mgr.token(), mgr.broadcaster.clone())
    };

    // Bind before reporting success so a busy port surfaces as an error
    // instead of a "running" badge on a server that never came up.
    let addr = SocketAddr::from(([0, 0, 0, 0], assigned_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind port {assigned_port}: {e}"))?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (token_tx, token_rx) = tokio::sync::watch::channel(assigned_token);
    let server_ctx = ServerContext {
        app: app.clone(),
        token: token_rx,
        broadcaster,
        shutdown: shutdown_rx.clone(),
    };

    let router = api::router(server_ctx);

    let task = tokio::spawn(async move {
        eprintln!("[web_gateway] running on http://{addr}");
        let mut graceful_rx = shutdown_rx.clone();
        let graceful = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = graceful_rx.wait_for(|stop| *stop).await;
        });
        let mut deadline_rx = shutdown_rx;
        let deadline = async move {
            let _ = deadline_rx.wait_for(|stop| *stop).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        };
        tokio::select! {
            res = graceful => {
                if let Err(e) = res {
                    eprintln!("[web_gateway] server error: {e}");
                }
            }
            _ = deadline => eprintln!("[web_gateway] forced shutdown after grace period"),
        }
        eprintln!("[web_gateway] stopped");
    });

    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    mgr.server = Some(RunningServer {
        shutdown_tx,
        token_tx,
        task,
    });
    Ok(mgr.status())
}

#[tauri::command]
pub async fn stop_gateway_cmd() -> Result<GatewayStatus, String> {
    let server = get_manager()
        .lock()
        .map_err(|e| e.to_string())?
        .server
        .take();
    if let Some(server) = server {
        let _ = server.shutdown_tx.send(true);
        // Await the task so the port is released before the UI can restart.
        let _ = server.task.await;
    }
    let mut mgr = get_manager().lock().map_err(|e| e.to_string())?;
    Ok(mgr.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_lan_ip_covers_only_rfc1918() {
        assert!(is_private_lan_ip("192.168.1.20"));
        assert!(is_private_lan_ip("10.0.0.5"));
        assert!(is_private_lan_ip("172.16.0.1"));
        assert!(is_private_lan_ip("172.31.255.254"));
        assert!(!is_private_lan_ip("172.15.0.1"));
        assert!(!is_private_lan_ip("172.32.0.1"));
        assert!(!is_private_lan_ip("100.64.0.1"));
        assert!(!is_private_lan_ip("not-an-ip"));
    }
}
