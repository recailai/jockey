use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::process::Stdio;
use std::sync::OnceLock;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use dashmap::DashMap;
use tauri::State;
use tokio::process::Command;

use crate::db::get_state;
use crate::types::AppState;

use super::cwd_util::resolve_cwd;

struct BuiltinAppTarget {
    display_name: &'static str,
    bundle_id: Option<&'static str>,
    icns_names: &'static [&'static str],
}

struct ResolvedApp {
    display_name: String,
    bundle_id: Option<String>,
    icns_names: &'static [&'static str],
    open_finder: bool,
}

const GENERIC_ICNS: &[&str] = &[
    "AppIcon.icns",
    "ApplicationIcon.icns",
    "app.icns",
    "icon.icns",
];

fn builtin_app_target(target: &str) -> Option<BuiltinAppTarget> {
    match target {
        "vscode" => Some(BuiltinAppTarget {
            display_name: "Visual Studio Code",
            bundle_id: Some("com.microsoft.VSCode"),
            icns_names: &[
                "Code.icns",
                "AppIcon.icns",
                "ApplicationIcon.icns",
                "app.icns",
                "icon.icns",
            ],
        }),
        "cursor" => Some(BuiltinAppTarget {
            display_name: "Cursor",
            bundle_id: Some("com.todesktop.230313mzl4w4u92"),
            icns_names: &[
                "Cursor.icns",
                "AppIcon.icns",
                "ApplicationIcon.icns",
                "app.icns",
                "icon.icns",
            ],
        }),
        "antigravity" => Some(BuiltinAppTarget {
            display_name: "Antigravity IDE",
            bundle_id: Some("com.google.antigravity-ide"),
            icns_names: &[
                "Antigravity IDE.icns",
                "ApplicationIcon.icns",
                "AppIcon.icns",
                "app.icns",
                "icon.icns",
            ],
        }),
        "terminal" => Some(BuiltinAppTarget {
            display_name: "Terminal",
            bundle_id: Some("com.apple.Terminal"),
            icns_names: &[
                "Terminal.icns",
                "AppIcon.icns",
                "ApplicationIcon.icns",
                "app.icns",
                "icon.icns",
            ],
        }),
        "finder" => Some(BuiltinAppTarget {
            display_name: "Finder",
            bundle_id: Some("com.apple.finder"),
            icns_names: &[
                "Finder.icns",
                "AppIcon.icns",
                "ApplicationIcon.icns",
                "app.icns",
                "icon.icns",
            ],
        }),
        _ => None,
    }
}

fn resolve_workspace_target(
    target: &str,
    app_name: Option<&str>,
    bundle_id: Option<&str>,
) -> Result<ResolvedApp, String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("workspace target required".to_string());
    }

    if target == "finder" {
        return Ok(ResolvedApp {
            display_name: "Finder".to_string(),
            bundle_id: Some("com.apple.finder".to_string()),
            icns_names: &[
                "Finder.icns",
                "AppIcon.icns",
                "ApplicationIcon.icns",
                "app.icns",
                "icon.icns",
            ],
            open_finder: true,
        });
    }

    if target == "custom" || target.starts_with("custom:") {
        let name = app_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "app name required for custom workspace app".to_string())?;
        let bundle = bundle_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return Ok(ResolvedApp {
            display_name: name.to_string(),
            bundle_id: bundle,
            icns_names: GENERIC_ICNS,
            open_finder: false,
        });
    }

    let builtin = builtin_app_target(target)
        .ok_or_else(|| format!("unsupported workspace target: {target}"))?;
    Ok(ResolvedApp {
        display_name: builtin.display_name.to_string(),
        bundle_id: builtin.bundle_id.map(str::to_string),
        icns_names: builtin.icns_names,
        open_finder: false,
    })
}

#[cfg(target_os = "macos")]
async fn mdfind_bundle(bundle_id: &str) -> Option<PathBuf> {
    let output = Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{bundle_id}'"))
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    pick_app_bundle(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
async fn mdfind_display_name(display_name: &str) -> Option<PathBuf> {
    let output = Command::new("mdfind")
        .arg(format!("kMDItemDisplayName == '{display_name}.app'"))
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    pick_app_bundle(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn pick_app_bundle(stdout: &str) -> Option<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.ends_with(".app"))
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn standard_app_paths(display_name: &str) -> Vec<PathBuf> {
    [
        format!("/Applications/{display_name}.app"),
        format!("/System/Applications/{display_name}.app"),
        format!("/Applications/{display_name} Preview.app"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(target_os = "macos")]
async fn find_app_bundle(app: &ResolvedApp) -> Option<PathBuf> {
    if let Some(id) = app.bundle_id.as_deref() {
        if let Some(path) = mdfind_bundle(id).await {
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    if let Some(path) = mdfind_display_name(&app.display_name).await {
        if path.is_dir() {
            return Some(path);
        }
    }
    for path in standard_app_paths(&app.display_name) {
        if path.is_dir() {
            return Some(path);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
async fn find_app_bundle(_app: &ResolvedApp) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
static ICON_DATA_URL_CACHE: OnceLock<DashMap<String, String>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn icon_data_url_cache() -> &'static DashMap<String, String> {
    ICON_DATA_URL_CACHE.get_or_init(DashMap::new)
}

#[cfg(target_os = "macos")]
fn icns_to_data_url(icns_path: &Path) -> Option<String> {
    let key = icns_path.to_string_lossy().into_owned();
    if let Some(cached) = icon_data_url_cache().get(&key) {
        return Some(cached.clone());
    }

    let out = std::env::temp_dir().join(format!("jockey-icon-{}.png", uuid::Uuid::new_v4()));
    let status = StdCommand::new("sips")
        .args([
            "-s",
            "format",
            "png",
            &icns_path.to_string_lossy(),
            "--out",
            &out.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let bytes = std::fs::read(&out).ok()?;
    let _ = std::fs::remove_file(&out);
    let url = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
    icon_data_url_cache().insert(key, url.clone());
    Some(url)
}

#[cfg(not(target_os = "macos"))]
fn icns_to_data_url(_icns_path: &Path) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn icon_from_resources(resources: &Path, names: &[&str]) -> Option<String> {
    for name in names {
        let icns = resources.join(name);
        if icns.is_file() {
            if let Some(url) = icns_to_data_url(&icns) {
                return Some(url);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(resources) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("icns") {
                if let Some(url) = icns_to_data_url(&path) {
                    return Some(url);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
async fn macos_app_icon_data_url(app: &ResolvedApp) -> Option<String> {
    let bundle = find_app_bundle(app).await?;
    let resources = bundle.join("Contents/Resources");
    icon_from_resources(&resources, app.icns_names)
}

#[cfg(not(target_os = "macos"))]
async fn macos_app_icon_data_url(_app: &ResolvedApp) -> Option<String> {
    None
}

#[tauri::command]
pub(crate) async fn get_workspace_app_icon_cmd(
    target: String,
    app_name: Option<String>,
    bundle_id: Option<String>,
) -> Result<Option<String>, String> {
    let app = resolve_workspace_target(&target, app_name.as_deref(), bundle_id.as_deref())?;
    Ok(macos_app_icon_data_url(&app).await)
}

#[tauri::command]
pub(crate) async fn open_workspace_cmd(
    state: State<'_, AppState>,
    app_session_id: Option<String>,
    target: String,
    app_name: Option<String>,
    bundle_id: Option<String>,
) -> Result<(), String> {
    let cwd = resolve_cwd(get_state(&state), app_session_id.as_deref());
    if !cwd.exists() {
        return Err(format!("workspace path does not exist: {}", cwd.display()));
    }

    let app = resolve_workspace_target(&target, app_name.as_deref(), bundle_id.as_deref())?;
    let mut cmd = Command::new("open");
    if app.open_finder {
        cmd.arg(&cwd);
    } else {
        cmd.args(["-a", &app.display_name]).arg(&cwd);
    }

    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("open workspace failed: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("open workspace failed with status: {status}"))
    }
}
