fn main() {
    let db_path = std::env::var("JOCKEYUI_DB_PATH").unwrap_or_else(|_| {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("com.jockeyui.app");
        dir.join("jockeyui.sqlite3").to_string_lossy().to_string()
    });

    if let Err(e) = jockeyui_lib::conductor_mcp::run_stdio_server(db_path) {
        eprintln!("jockeyui-mcp fatal: {e}");
        std::process::exit(1);
    }
}
