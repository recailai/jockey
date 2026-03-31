fn main() {
    // Re-run this build script (and re-link the binary) whenever the frontend
    // dist output changes, so `pnpm tauri build` always embeds the latest UI.
    println!("cargo:rerun-if-changed=../dist");
    tauri_build::build()
}
