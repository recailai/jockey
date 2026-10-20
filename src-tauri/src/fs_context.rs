
pub(crate) fn resolve_attach_path(cwd: &str, raw: &str) -> std::path::PathBuf {
    let input = raw.trim();
    let expanded = if input == "~" || input.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if input == "~" {
            home
        } else {
            format!("{home}{}", &input[1..])
        }
    } else {
        input.to_string()
    };
    let candidate = std::path::PathBuf::from(expanded);
    let joined = if candidate.is_absolute() {
        candidate
    } else {
        std::path::PathBuf::from(cwd).join(candidate)
    };
    joined.canonicalize().unwrap_or(joined)
}

fn workspace_root(cwd: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(cwd)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(cwd))
}

pub(crate) fn is_within_workspace(path: &std::path::Path, cwd: &str) -> bool {
    path.starts_with(workspace_root(cwd))
}

pub(crate) fn relative_or_abs(path: &std::path::Path, cwd: &str) -> String {
    let cwd_path = std::path::Path::new(cwd);
    if let Ok(relative) = path.strip_prefix(cwd_path) {
        return relative.to_string_lossy().to_string();
    }
    path.to_string_lossy().to_string()
}

pub(crate) fn should_skip_name(name: &str, show_hidden: bool) -> bool {
    if name.starts_with('.') {
        return !show_hidden;
    }
    matches!(name, "node_modules" | "dist" | "build" | "target")
}

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let mut suspicious = 0usize;
    for b in bytes {
        let is_control = (*b < 0x09) || (*b > 0x0D && *b < 0x20);
        if is_control {
            suspicious += 1;
        }
    }
    suspicious * 10 > bytes.len()
}
