//! Model/effort catalog discovery for headless (`--print`) runtimes.
//!
//! Headless CLIs have no control plane to query at runtime, so there is nothing
//! equivalent to Codex's `model/list`. Each protocol gets the best source it has:
//!
//! - Antigravity exposes a real `agy models` subcommand, so its catalog is live.
//! - Claude Code has no listing command, so it uses a curated manifest gated on the
//!   installed CLI version, merged with whatever the user pinned in `settings.json`.
//!   Version gating is what keeps the list honest: a model only appears once the
//!   locally installed `claude` is new enough to route it.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::process::Command;

use super::super::super::adapter::{acp_log, HeadlessProtocol};
use super::super::super::runtime_state::{
    clear_runtime, fresh_config_options, remember_runtime_config_options, remember_runtime_models,
};
use super::super::option_spec::{OptionSpec, OptionValue, Wire};

const CATALOG_TIMEOUT: Duration = Duration::from_secs(20);

pub(in crate::acp) struct HeadlessCatalog {
    pub(in crate::acp) options: Vec<Value>,
    pub(in crate::acp) modes: Vec<String>,
}

struct ClaudeModel {
    id: &'static str,
    label: &'static str,
    /// Short qualifier shown next to the label in the picker; empty when it adds nothing.
    blurb: &'static str,
    /// Lowest `claude --version` that can route this model. `None` means every
    /// version this adapter accepts already supports it.
    min_version: Option<(u32, u32, u32)>,
    /// Has a `<id>[1m]` sibling. The picker exposes this as a toggle rather than as a second
    /// model entry — a 1M session is the same model with a bigger window, and listing both
    /// doubles the menu for no decision the user actually makes separately.
    one_million: bool,
    /// Accepts `--settings '{"fastMode":true}'`.
    supports_fast: bool,
    effort_levels: &'static [&'static str],
}

/// Only a last resort for when `claude --help` cannot be read; the real list is parsed from
/// the installed CLI (`--effort <level> ... (low, medium, high, xhigh, max)`), so it tracks
/// whatever that build actually accepts instead of a constant we have to remember to update.
const EFFORT_FALLBACK: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const EFFORT_NONE: &[&str] = &[];

/// Newest first — the picker preserves this order, and a bare alias like `opus` resolves to
/// the first match.
const CLAUDE_MODELS: &[ClaudeModel] = &[
    ClaudeModel {
        id: "claude-opus-5",
        label: "Opus 5",
        blurb: "most capable",
        min_version: Some((2, 1, 219)),
        one_million: true,
        supports_fast: true,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-fable-5-1",
        label: "Fable 5.1",
        blurb: "",
        min_version: Some((2, 1, 219)),
        one_million: false,
        supports_fast: false,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-sonnet-5",
        label: "Sonnet 5",
        blurb: "everyday",
        min_version: Some((2, 1, 169)),
        one_million: true,
        supports_fast: false,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-haiku-4-5",
        label: "Haiku 4.5",
        blurb: "fastest",
        min_version: None,
        one_million: false,
        supports_fast: false,
        effort_levels: EFFORT_NONE,
    },
    ClaudeModel {
        id: "claude-fable-5",
        label: "Fable 5",
        blurb: "",
        min_version: Some((2, 1, 169)),
        one_million: false,
        supports_fast: false,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-opus-4-8",
        label: "Opus 4.8",
        blurb: "",
        min_version: None,
        one_million: true,
        supports_fast: true,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-opus-4-7",
        label: "Opus 4.7",
        blurb: "",
        min_version: None,
        one_million: true,
        supports_fast: true,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-opus-4-6",
        label: "Opus 4.6",
        blurb: "",
        min_version: None,
        one_million: true,
        supports_fast: true,
        effort_levels: EFFORT_FALLBACK,
    },
    ClaudeModel {
        id: "claude-sonnet-4-6",
        label: "Sonnet 4.6",
        blurb: "",
        min_version: None,
        one_million: true,
        supports_fast: false,
        effort_levels: EFFORT_FALLBACK,
    },
];

/// Antigravity documents `--effort (low|medium|high)`, so it does have its own effort axis —
/// the `-high`/`-medium`/`-low` suffixes on its model ids are a separate dimension, not a
/// replacement for it. Read from `--help` at discovery time; this is only the fallback.
const AGY_EFFORT_LEVELS: &[&str] = &["low", "medium", "high"];

/// One entry in the picker's Model submenu.
struct ModelEntry {
    id: String,
    label: String,
    blurb: String,
    one_million: bool,
    supports_fast: bool,
    effort_levels: Vec<String>,
    /// The effort this model actually runs at when the user has pinned nothing — taken from
    /// their own `settings.json` `modelSettings` when present, else the manifest default.
    /// Without this the picker shows a blank Effort row even though the CLI has a real one.
    default_effort: Option<String>,
    /// The model the runtime picks when nothing is selected, so the picker can name it
    /// instead of rendering an opaque "default: runtime".
    is_default: bool,
}

impl ModelEntry {
    fn to_value(&self) -> Value {
        json!({
            "value": self.id,
            "name": self.label,
            "description": self.blurb,
            // Per-model capabilities drive which rows the picker shows (1M / Fast) and which
            // effort levels are offered, so they travel with the model rather than the runtime.
            "oneMillion": self.one_million,
            "supportsFast": self.supports_fast,
            "effortLevels": self.effort_levels,
            "defaultEffort": self.default_effort,
            "isDefault": self.is_default,
        })
    }
}

pub(in crate::acp) async fn refresh_headless_catalog(
    protocol: HeadlessProtocol,
    runtime_key: &'static str,
    binary: &str,
    env: &[(String, String)],
    cwd: &str,
    force_refresh: bool,
) -> HeadlessCatalog {
    // Discovery costs a subprocess (and a network round-trip for `agy models`), while
    // prewarm runs on every session resume. Reuse the process-lifetime catalog unless the
    // caller explicitly asked for a refresh.
    // Same rule as the native path: reuse only a complete, still-fresh catalog. Other paths
    // (a cold start whose provider reported no config options) remember models alone, and
    // reusing on that basis handed the picker an empty option set.
    if !force_refresh {
        if let Some(options) = fresh_config_options(runtime_key) {
            return HeadlessCatalog {
                options,
                modes: vec![],
            };
        }
    }
    clear_runtime(runtime_key);
    let models = match protocol {
        HeadlessProtocol::ClaudeStreamJson => claude_models(binary, env, cwd).await,
        HeadlessProtocol::AgyStreamJson => {
            // `agy --help` documents `--effort (low|medium|high)`; the const is only a fallback.
            let levels = super::super::cli_help::levels_from_help(binary, env, cwd, "--effort")
                .await
                .unwrap_or_else(|| AGY_EFFORT_LEVELS.iter().map(|s| s.to_string()).collect());
            agy_models(binary, env, cwd, &levels).await
        }
    };
    acp_log(
        "headless.catalog",
        json!({ "runtime": runtime_key, "models": models.len() }),
    );
    if models.is_empty() {
        return HeadlessCatalog {
            options: vec![],
            modes: vec![],
        };
    }
    // The `[1m]` variants are reachable through the per-model toggle, but they are still
    // valid ids a role may already hold, so they stay in the known-models list.
    let mut known_models: Vec<String> = Vec::with_capacity(models.len());
    for model in &models {
        known_models.push(model.id.clone());
        if model.one_million {
            known_models.push(format!("{}[1m]", model.id));
        }
    }
    remember_runtime_models(runtime_key, known_models);
    let options = build_options(&models, protocol);
    remember_runtime_config_options(runtime_key, options.clone());
    HeadlessCatalog {
        options,
        modes: vec![],
    }
}

/// The boolean knobs, each declaring how it actually reaches the CLI. The two Claude knobs
/// look identical to the user and are delivered by completely different mechanisms — 1M
/// context by suffixing the model id, fast mode by a `--settings` blob — which is exactly why
/// the mechanism belongs in the declaration rather than in a branch on the send path.
///
/// Driven by what the models advertise, so a runtime that offers neither (Antigravity) emits
/// neither, without anyone having to special-case it here.
fn toggle_options(models: &[ModelEntry]) -> Vec<Value> {
    let mut out = Vec::new();
    let one_million: Vec<String> = models
        .iter()
        .filter(|m| m.one_million)
        .map(|m| m.id.clone())
        .collect();
    if !one_million.is_empty() {
        out.push(
            OptionSpec::toggle(
                "one_million",
                "1M context",
                Wire::ModelSuffix {
                    suffix: "[1m]".to_string(),
                },
            )
            .describe("Run this model with a 1M-token context window")
            .for_models(one_million)
            .to_value(),
        );
    }
    let fast: Vec<String> = models
        .iter()
        .filter(|m| m.supports_fast)
        .map(|m| m.id.clone())
        .collect();
    if !fast.is_empty() {
        out.push(
            OptionSpec::toggle(
                "fast",
                "Fast mode",
                Wire::CliSettings {
                    json: json!({ "fastMode": true }),
                },
            )
            .describe("Faster output at the same model quality")
            .for_models(fast)
            .to_value(),
        );
    }
    out
}

/// The runtime-wide effort list, taken from what the models themselves reported — those levels
/// came from `--help` on the installed binary. Building it from the constant instead, as this
/// did, meant the runtime-level list silently ignored the probe and could disagree with the
/// per-model lists sitting beside it.
fn effort_values(models: &[ModelEntry], protocol: HeadlessProtocol) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for model in models {
        for level in &model.effort_levels {
            if seen.insert(level.clone()) {
                out.push(level.clone());
            }
        }
    }
    if out.is_empty() {
        out = match protocol {
            HeadlessProtocol::ClaudeStreamJson => {
                EFFORT_FALLBACK.iter().map(|s| s.to_string()).collect()
            }
            HeadlessProtocol::AgyStreamJson => {
                AGY_EFFORT_LEVELS.iter().map(|s| s.to_string()).collect()
            }
        };
    }
    out
}

/// The effort the runtime itself would use: the default model's, else any model that knows one.
fn runtime_default_effort(models: &[ModelEntry]) -> Option<String> {
    models
        .iter()
        .find(|m| m.is_default)
        .and_then(|m| m.default_effort.clone())
        .or_else(|| models.iter().find_map(|m| m.default_effort.clone()))
}

fn build_options(models: &[ModelEntry], protocol: HeadlessProtocol) -> Vec<Value> {
    let model_option = OptionSpec::select(
        "model",
        "Model",
        Wire::CliFlag {
            flag: "--model".to_string(),
        },
        models
            .iter()
            // Capabilities belong to the model, not to the "model" parameter: which effort
            // levels it supports, whether it offers a 1M window or a faster tier, what it
            // defaults to. They ride along as this value's own metadata.
            .map(|m| {
                OptionValue::new(m.id.clone(), m.label.clone())
                    .described(m.blurb.clone())
                    .with_meta(m.to_value())
            })
            .collect(),
    )
    .describe("Discovered from the installed CLI")
    .defaulting_to(models.iter().find(|m| m.is_default).map(|m| m.id.clone()))
    .to_value();

    let mut out = vec![model_option];

    let levels = effort_values(models, protocol);
    if !levels.is_empty() {
        out.push(
            OptionSpec::select(
                "effort",
                "Effort",
                Wire::CliFlag {
                    flag: "--effort".to_string(),
                },
                levels
                    .iter()
                    .map(|level| OptionValue::new(level.clone(), level.clone()))
                    .collect(),
            )
            .describe("Reasoning effort for the session (--effort)")
            .defaulting_to(runtime_default_effort(models))
            .to_value(),
        );
    }

    out.extend(toggle_options(models));
    out
}

/// `claude-opus-5` / `opus` / `opus[1m]` all belong to the `opus` family. Used to line the
/// manifest up with the alias menu the CLI actually offers.
fn model_family(id: &str) -> Option<String> {
    let stripped = id.trim().trim_end_matches("[1m]");
    let core = stripped.strip_prefix("claude-").unwrap_or(stripped);
    let family = core.split('-').next()?.trim();
    if family.is_empty() {
        None
    } else {
        Some(family.to_ascii_lowercase())
    }
}

async fn claude_models(binary: &str, env: &[(String, String)], cwd: &str) -> Vec<ModelEntry> {
    let version = claude_version(binary, env, cwd).await;
    // Ask the CLI what it is actually set to rather than inferring it. Costs no tokens.
    let current = claude_current_model(binary, env, cwd)
        .await
        .unwrap_or_default();
    // Levels the installed binary documents; the constant is only a fallback.
    let levels = super::super::cli_help::levels_from_help(binary, env, cwd, "--effort").await;
    let effort_for = |fallback: &[&str]| -> Vec<String> {
        levels
            .clone()
            .unwrap_or_else(|| fallback.iter().map(|s| s.to_string()).collect())
    };
    let mut models: Vec<ModelEntry> = CLAUDE_MODELS
        .iter()
        .filter(|model| match (model.min_version, version) {
            (Some(required), Some(installed)) => installed >= required,
            // An unreadable `--version` must not silently hide models the user has.
            (Some(_), None) => true,
            (None, _) => true,
        })
        .map(|model| ModelEntry {
            id: model.id.to_string(),
            label: model.label.to_string(),
            blurb: model.blurb.to_string(),
            one_million: model.one_million,
            supports_fast: model.supports_fast,
            effort_levels: if model.effort_levels.is_empty() {
                Vec::new()
            } else {
                effort_for(model.effort_levels)
            },
            default_effort: default_effort_for(model.effort_levels),
            is_default: false,
        })
        .collect();

    // The CLI's `Available:` list is the real menu, and it offers one alias per family
    // (`opus`, `sonnet`, …) pointing at that family's newest model. Older dated builds are
    // reachable only by full id, which we cannot verify — so keep the newest entry of each
    // offered family and drop the rest. That is what the hand-maintained "previous" tier was
    // standing in for, and it was wrong as often as it was right.
    if !current.aliases.is_empty() {
        let offered: BTreeSet<String> = current
            .aliases
            .iter()
            .filter_map(|alias| model_family(alias.trim_end_matches("[1m]")))
            .collect();
        let mut kept_families: BTreeSet<String> = BTreeSet::new();
        models.retain(|model| {
            let Some(family) = model_family(&model.id) else {
                return true;
            };
            if !offered.contains(&family) {
                return false;
            }
            kept_families.insert(family)
        });
    }

    // Mark whatever the CLI reported as current; fall back to the first entry only when the
    // probe could not run.
    let default_idx = models
        .iter()
        .position(|m| !current.label.is_empty() && m.label == current.label)
        .or_else(|| models.iter().position(|_| true));
    if let Some(idx) = default_idx {
        models[idx].is_default = true;
        if let Some(effort) = current.effort.clone() {
            models[idx].default_effort = Some(effort);
        }
    }
    let known: BTreeSet<String> = models.iter().map(|m| m.id.clone()).collect();
    for pinned in claude_settings_models() {
        if known.contains(&pinned) {
            continue;
        }
        // A pinned id is a real routing target the manifest cannot describe, so it gets the
        // permissive effort list and no capability toggles.
        models.push(ModelEntry {
            id: pinned.clone(),
            label: pinned,
            blurb: "from settings.json".to_string(),
            one_million: false,
            supports_fast: false,
            effort_levels: effort_for(EFFORT_FALLBACK),
            default_effort: current.effort.clone(),
            is_default: false,
        });
    }
    models
}

/// What the CLI itself reports for `/model`. This is a *local* slash command: no tokens, no
/// API turn (~30 ms), and it answers with the model and effort Claude Code has actually
/// resolved — settings.json, env overrides and account entitlements already applied. Parsing
/// settings.json ourselves only re-derives a worse version of this.
#[derive(Debug, Default, Clone, PartialEq)]
struct ClaudeCurrentModel {
    /// Display label as the CLI prints it, e.g. `Opus 5`.
    label: String,
    /// True when the CLI reported the 1M-context variant.
    one_million: bool,
    /// Effort the CLI is running at, e.g. `high`.
    effort: Option<String>,
    /// Aliases `/model <name>` accepts on this install.
    aliases: Vec<String>,
}

/// Parses: ``Current model: `Opus 5 (1M context)` (effort: high)\nUsage: /model <name>.
/// Available: sonnet, opus, … or a full model ID.``
fn parse_claude_model_command(result: &str) -> Option<ClaudeCurrentModel> {
    let mut out = ClaudeCurrentModel::default();

    if let Some(rest) = result.split("Current model:").nth(1) {
        if let Some(label) = rest.split('`').nth(1) {
            let label = label.trim();
            let (base, one_million) = match label.strip_suffix("(1M context)") {
                Some(stripped) => (stripped.trim(), true),
                None => (label, false),
            };
            out.label = base.to_string();
            out.one_million = one_million;
        }
        if let Some(effort) = rest
            .split("(effort:")
            .nth(1)
            .and_then(|e| e.split(')').next())
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            out.effort = Some(effort.to_string());
        }
    }

    if let Some(list) = result.split("Available:").nth(1) {
        out.aliases = list
            .split(" or a full model ID")
            .next()
            .unwrap_or(list)
            .split(',')
            .map(|a| a.trim().trim_end_matches('.').to_string())
            .filter(|a| !a.is_empty() && !a.contains(' '))
            .collect();
    }

    if out.label.is_empty() && out.aliases.is_empty() {
        return None;
    }
    Some(out)
}

async fn claude_current_model(
    binary: &str,
    env: &[(String, String)],
    cwd: &str,
) -> Option<ClaudeCurrentModel> {
    let probe_dir = std::env::temp_dir();
    let probe_cwd = probe_dir.to_str().unwrap_or(cwd);
    let output = run_capture(
        binary,
        &["--print", "/model", "--output-format", "json"],
        env,
        probe_cwd,
    )
    .await?;
    let parsed = serde_json::from_str::<Value>(&output).ok()?;
    let result = parsed.get("result").and_then(Value::as_str)?;
    parse_claude_model_command(result)
}

async fn claude_version(
    binary: &str,
    env: &[(String, String)],
    cwd: &str,
) -> Option<(u32, u32, u32)> {
    let probe_dir = std::env::temp_dir();
    let probe_cwd = probe_dir.to_str().unwrap_or(cwd);
    let output = run_capture(binary, &["--version"], env, probe_cwd).await?;
    parse_claude_version(&output)
}

/// `2.1.273 (Claude Code)` / `v10.0.3` — the leading dotted triple, however it is decorated.
fn parse_claude_version(banner: &str) -> Option<(u32, u32, u32)> {
    let token = banner
        .split_whitespace()
        .find(|token| {
            token
                .trim_start_matches('v')
                .starts_with(|c: char| c.is_ascii_digit())
        })?
        .trim_start_matches('v');
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .map(|p| p.trim_matches(|c: char| !c.is_ascii_digit()))
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// Fallback when the CLI probe is unavailable: Claude Code routes thinking at `high`.
fn default_effort_for(effort_levels: &[&str]) -> Option<String> {
    effort_levels
        .iter()
        .find(|level| **level == "high")
        .map(|level| level.to_string())
}

/// Models the user pinned in `~/.claude/settings.json`, either as `model` or via the
/// `ANTHROPIC_*_MODEL` env overrides. These are real routing targets the manifest
/// cannot know about (gateways, Bedrock ids, custom providers).
fn claude_settings_models() -> Vec<String> {
    const ENV_KEYS: &[&str] = &[
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ];
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|home| home.join(".claude").to_string_lossy().to_string())
            .unwrap_or_default()
    });
    if config_dir.is_empty() {
        return vec![];
    }
    let Ok(raw) = std::fs::read_to_string(std::path::Path::new(&config_dir).join("settings.json"))
    else {
        return vec![];
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut push = |value: Option<&Value>| {
        if let Some(id) = value.and_then(Value::as_str).map(str::trim) {
            if !id.is_empty() && !out.iter().any(|existing| existing == id) {
                out.push(id.to_string());
            }
        }
    };
    push(parsed.get("model"));
    for key in ENV_KEYS {
        push(parsed.get("env").and_then(|env| env.get(key)));
    }
    out
}

/// `agy models` prints `<id>\t<label>` per line after a status header.
async fn agy_models(
    binary: &str,
    env: &[(String, String)],
    cwd: &str,
    effort_levels: &[String],
) -> Vec<ModelEntry> {
    let Some(output) = run_capture(binary, &["models"], env, cwd).await else {
        return vec![];
    };
    parse_agy_models(&output, effort_levels)
}

fn parse_agy_models(output: &str, effort_levels: &[String]) -> Vec<ModelEntry> {
    output
        .lines()
        .filter_map(|line| {
            let (id, label) = line.split_once('\t')?;
            let id = id.trim();
            let label = label.trim();
            if id.is_empty() || id.contains(' ') {
                return None;
            }
            Some(ModelEntry {
                id: id.to_string(),
                label: if label.is_empty() {
                    id.to_string()
                } else {
                    label.to_string()
                },
                blurb: String::new(),
                one_million: false,
                supports_fast: false,
                effort_levels: effort_levels.to_vec(),
                // The listing is live but says nothing about defaults.
                default_effort: effort_levels.iter().find(|level| *level == "high").cloned(),
                is_default: false,
            })
        })
        .collect()
}

async fn run_capture(
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
    cwd: &str,
) -> Option<String> {
    let mut cmd = Command::new(binary);
    cmd.args(args).kill_on_drop(true);
    if !cwd.trim().is_empty() {
        cmd.current_dir(cwd);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = tokio::time::timeout(CATALOG_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_offered_alias_list_replaces_the_hand_written_previous_tier() {
        // Real aliases from `/model` on 2.1.273. The CLI offers one alias per family, so only
        // each family's newest build is actually reachable — Opus 4.8/4.7/4.6 and Sonnet 4.6
        // were never verifiable, which is exactly what the "previous" tier was hiding.
        for (id, family) in [
            ("claude-opus-5", "opus"),
            ("claude-sonnet-5", "sonnet"),
            ("claude-haiku-4-5", "haiku"),
            ("claude-fable-5-1", "fable"),
            ("opus[1m]", "opus"),
            ("opus", "opus"),
        ] {
            assert_eq!(super::model_family(id).as_deref(), Some(family), "{id}");
        }
    }

    #[test]
    fn reads_the_live_model_and_effort_the_cli_reports() {
        // Verbatim `result` from `claude --print "/model" --output-format json` on 2.1.273.
        // This is a local slash command: no tokens, no API turn — which is what makes it a
        // better source for defaults than re-deriving them from settings.json.
        let raw = "Current model: `Opus 5 (1M context)` (effort: high)\nUsage: /model <name>. \
Available: sonnet, opus, haiku, fable, best, sonnet[1m], opus[1m], fable[1m], opusplan, \
default, or a full model ID.";
        let parsed = super::parse_claude_model_command(raw).expect("parses");
        assert_eq!(parsed.label, "Opus 5");
        assert!(parsed.one_million);
        assert_eq!(parsed.effort.as_deref(), Some("high"));
        assert!(parsed.aliases.contains(&"opus[1m]".to_string()));
        assert!(parsed.aliases.contains(&"opusplan".to_string()));
        assert!(!parsed.aliases.iter().any(|a| a.contains("full model")));
    }

    #[test]
    fn a_model_line_without_effort_still_parses() {
        let parsed = super::parse_claude_model_command(
            "Current model: `Sonnet 5`\nUsage: /model <name>. Available: sonnet, opus.",
        )
        .expect("parses");
        assert_eq!(parsed.label, "Sonnet 5");
        assert!(!parsed.one_million);
        assert_eq!(parsed.effort, None);
        assert_eq!(
            parsed.aliases,
            vec!["sonnet".to_string(), "opus".to_string()]
        );
    }

    use super::*;

    #[test]
    fn parses_claude_version_banner() {
        assert_eq!(
            parse_claude_version("2.1.272 (Claude Code)"),
            Some((2, 1, 272))
        );
        assert_eq!(parse_claude_version("v10.0.3\n"), Some((10, 0, 3)));
        assert_eq!(parse_claude_version("no version here"), None);
    }

    #[test]
    fn parses_agy_models_listing() {
        let out = "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n";
        let models = parse_agy_models(out, &["low".to_string(), "high".to_string()]);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gemini-3.8-flash-high");
        assert_eq!(models[1].label, "Claude Sonnet 4.6 (Thinking)");
    }

    fn claude_test_models() -> Vec<ModelEntry> {
        vec![
            ModelEntry {
                id: "claude-opus-5".to_string(),
                label: "Opus 5".to_string(),
                blurb: "most capable".to_string(),
                one_million: true,
                supports_fast: true,
                effort_levels: vec!["low".to_string(), "high".to_string()],
                default_effort: Some("high".to_string()),
                is_default: true,
            },
            ModelEntry {
                id: "claude-haiku-4-5".to_string(),
                label: "Haiku 4.5".to_string(),
                blurb: String::new(),
                one_million: false,
                supports_fast: false,
                effort_levels: vec![],
                default_effort: None,
                is_default: false,
            },
        ]
    }

    #[test]
    fn claude_declares_its_two_knobs_with_their_different_delivery_mechanisms() {
        // The user sees two identical-looking switches; on the wire one suffixes the model id
        // and the other injects a `--settings` blob. The send path must read that from the
        // declaration, not re-derive it from the option's name.
        let options = build_options(&claude_test_models(), HeadlessProtocol::ClaudeStreamJson);
        let find = |id: &str| {
            options
                .iter()
                .find(|o| o["id"] == id)
                .cloned()
                .unwrap_or_else(|| panic!("{id} missing"))
        };

        let one_million = find("one_million");
        assert_eq!(one_million["kind"], "toggle");
        assert_eq!(one_million["wire"]["kind"], "model_suffix");
        assert_eq!(one_million["wire"]["suffix"], "[1m]");

        let fast = find("fast");
        assert_eq!(fast["kind"], "toggle");
        assert_eq!(fast["wire"]["kind"], "cli_settings");
        assert_eq!(fast["wire"]["json"]["fastMode"], true);

        // Haiku advertises neither, so neither knob claims it.
        assert_eq!(one_million["appliesToModels"], json!(["claude-opus-5"]));
        assert_eq!(fast["appliesToModels"], json!(["claude-opus-5"]));
    }

    #[test]
    fn the_model_option_keeps_every_per_model_capability_the_picker_reads() {
        let options = build_options(&claude_test_models(), HeadlessProtocol::ClaudeStreamJson);
        let model = options.iter().find(|o| o["id"] == "model").expect("model");

        // Headless runtimes are configured when the process launches; they have no per-turn
        // channel, so the declaration says flag, not turn parameter.
        assert_eq!(model["wire"]["kind"], "cli_flag");
        assert_eq!(model["wire"]["flag"], "--model");
        assert_eq!(model["default"], "claude-opus-5");
        assert_eq!(model["options"][0]["oneMillion"], true);
        assert_eq!(model["options"][0]["supportsFast"], true);
        assert_eq!(model["options"][0]["effortLevels"][1], "high");
        assert_eq!(model["options"][0]["isDefault"], true);
        // Legacy consumers match on these, so they have to survive the migration.
        assert_eq!(model["type"], "select");
        assert_eq!(model["category"], "model");
    }

    #[test]
    fn the_runtime_effort_list_comes_from_the_models_not_the_constant() {
        let options = build_options(&claude_test_models(), HeadlessProtocol::ClaudeStreamJson);
        let effort = options
            .iter()
            .find(|o| o["id"] == "effort")
            .expect("effort");

        assert_eq!(effort["wire"]["flag"], "--effort");
        assert_eq!(effort["default"], "high");
        assert_eq!(
            effort["options"],
            json!([{ "value": "low", "name": "low" }, { "value": "high", "name": "high" }]),
            "the constant would have added medium/xhigh/max the probe never reported"
        );
    }

    #[test]
    fn antigravity_declares_no_boolean_knobs() {
        let models = parse_agy_models(
            "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n",
            &["low".to_string(), "high".to_string()],
        );
        let options = build_options(&models, HeadlessProtocol::AgyStreamJson);
        let ids: Vec<&str> = options.iter().filter_map(|o| o["id"].as_str()).collect();
        assert_eq!(
            ids,
            vec!["model", "effort"],
            "agy advertises no fast or 1M knob, and `agy --help` has no such flag"
        );
    }

    #[test]
    fn version_gate_hides_models_the_installed_cli_cannot_route() {
        let gated = CLAUDE_MODELS
            .iter()
            .filter(|m| match m.min_version {
                Some(required) => (2, 1, 100) >= required,
                None => true,
            })
            .any(|m| m.id == "claude-opus-5");
        assert!(!gated, "opus-5 must be hidden on claude 2.1.100");
    }
}
