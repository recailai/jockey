#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Ok(path) = std::env::var("PATH") {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut new_path = format!(
            "/usr/local/bin:{home}/.npm-global/bin:{home}/.bun/bin:{home}/.cargo/bin:{path}"
        );
        if cfg!(target_os = "macos") {
            new_path = format!("/opt/homebrew/bin:{new_path}");
        }
        #[allow(deprecated)]
        std::env::set_var("PATH", new_path);
    }
    // Internal stdio MCP permission bridge used by the native Claude adapter.
    // Must be checked before the Tauri app starts.
    if std::env::args().any(|arg| arg == "--__jockey-permission-bridge") {
        jockey_lib::permission_bridge_main();
        return;
    }
    jockey_lib::run()
}
