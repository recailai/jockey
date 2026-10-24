use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const COMPANION_DIST: &str = "../dist-companion";
const PLACEHOLDER: &str = "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Jockey companion</title><body style=\"font-family:system-ui;padding:24px\"><h3>Companion UI not built</h3><p>Run <code>pnpm build</code> (or <code>pnpm build:companion</code>) and rebuild the app.</p></body>";

fn collect(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), path));
        }
    }
}

/// The companion page is built by Vite into `dist-companion/` and compiled into the binary.
/// Without that directory (plain `cargo check/test`) a placeholder page keeps the crate
/// buildable.
fn embed_companion() {
    println!("cargo:rerun-if-changed={COMPANION_DIST}");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let base = manifest.join(COMPANION_DIST);
    let mut files = Vec::new();
    collect(&base, &base, &mut files);
    if !files.iter().any(|(rel, _)| rel == "index.html") {
        files.clear();
    }

    let mut code = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    if files.is_empty() {
        let _ = writeln!(code, "    (\"index.html\", {PLACEHOLDER:?}.as_bytes()),");
    }
    for (rel, path) in &files {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        let _ = writeln!(
            code,
            "    ({rel:?}, include_bytes!({:?})),",
            abs.to_string_lossy()
        );
    }
    code.push_str("];\n");
    let out =
        PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("companion_assets.rs");
    std::fs::write(out, code).expect("write companion_assets.rs");
}

fn main() {
    // Re-run this build script (and re-link the binary) whenever the frontend
    // dist output changes, so `pnpm tauri build` always embeds the latest UI.
    println!("cargo:rerun-if-changed=../dist");
    embed_companion();
    tauri_build::build()
}
