//! What each agent CLI needs to run under a different account.
//!
//! Isolation is always additive: a profile only ever points a CLI at *another* config dir
//! or adds env vars. The CLI's default dir and the OS keychain items it already owns are
//! never written, so binding or deleting profiles cannot sign the user out of their
//! existing login.

use crate::runtime_kind::RuntimeKind;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialProvider {
    Claude,
    Codex,
    Pi,
    Antigravity,
}

pub(crate) const ALL_PROVIDERS: [CredentialProvider; 4] = [
    CredentialProvider::Claude,
    CredentialProvider::Codex,
    CredentialProvider::Pi,
    CredentialProvider::Antigravity,
];

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStatus {
    pub(crate) signed_in: bool,
    pub(crate) identity: Option<String>,
}

impl CredentialProvider {
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "antigravity" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Antigravity => "antigravity",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex CLI",
            Self::Pi => "Pi CLI",
            Self::Antigravity => "Antigravity CLI",
        }
    }

    /// Both Claude runtimes (headless and the ACP bridge) run the same Claude Code install,
    /// so they share one account.
    pub(crate) fn for_runtime(runtime_kind: &str) -> Option<Self> {
        match RuntimeKind::from_str(runtime_kind)? {
            RuntimeKind::ClaudeNative | RuntimeKind::ClaudeCode => Some(Self::Claude),
            RuntimeKind::CodexCli => Some(Self::Codex),
            RuntimeKind::PiCli => Some(Self::Pi),
            RuntimeKind::AntigravityCli => Some(Self::Antigravity),
            RuntimeKind::Mock => None,
        }
    }

    /// Env var that relocates the CLI's config and auth state. `None` when the CLI keeps its
    /// login somewhere a profile cannot redirect: agy stores its OAuth token in one fixed
    /// keychain item (`gemini`/`antigravity`), so a second sign-in would overwrite the user's
    /// current account. Antigravity profiles are therefore env-only (API key or ADC).
    pub(crate) fn config_dir_env(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("CLAUDE_CONFIG_DIR"),
            Self::Codex => Some("CODEX_HOME"),
            Self::Pi => Some("PI_CODING_AGENT_DIR"),
            Self::Antigravity => None,
        }
    }

    pub(crate) fn default_config_dir(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        match self {
            Self::Claude => Some(home.join(".claude")),
            Self::Codex => Some(home.join(".codex")),
            Self::Pi => Some(home.join(".pi").join("agent")),
            Self::Antigravity => Some(home.join(".gemini")),
        }
    }

    /// Entries linked from the default dir into a Jockey-managed one so a second account
    /// keeps the same settings, extensions and conversation history (resume and transcript
    /// import keep working). Credential files and per-install databases are deliberately
    /// absent.
    fn shared_entries(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &[
                "settings.json",
                "CLAUDE.md",
                "projects",
                "agents",
                "commands",
                "skills",
                "plugins",
                "hooks",
                "output-styles",
            ],
            Self::Codex => &[
                "config.toml",
                "AGENTS.md",
                "sessions",
                "archived_sessions",
                "prompts",
                "rules",
                "skills",
                "plugins",
            ],
            Self::Pi => &[
                "settings.json",
                "models.json",
                "mcp.json",
                "AGENTS.md",
                "SYSTEM.md",
                "extensions",
                "skills",
                "prompts",
                "themes",
                "npm",
                "bin",
            ],
            Self::Antigravity => &[],
        }
    }

    /// Extra env a managed dir needs to keep sharing state the CLI can relocate directly.
    pub(crate) fn managed_env(self) -> Vec<(String, String)> {
        match (self, self.default_config_dir()) {
            (Self::Pi, Some(dir)) => vec![(
                "PI_CODING_AGENT_SESSION_DIR".to_string(),
                dir.join("sessions").to_string_lossy().into_owned(),
            )],
            _ => Vec::new(),
        }
    }

    /// Shell command that signs in under the profile's config dir.
    pub(crate) fn login_command(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude auth login"),
            Self::Codex => Some("codex login"),
            // Pi has no standalone login subcommand; `/login` inside the TUI picks a provider.
            Self::Pi => Some("pi"),
            Self::Antigravity => None,
        }
    }

    pub(crate) fn login_hint(self) -> &'static str {
        match self {
            Self::Claude => "Completes the Claude sign-in in your browser.",
            Self::Codex => "Completes the ChatGPT sign-in in your browser.",
            Self::Pi => "Type /login in Pi and pick a provider, then quit Pi.",
            Self::Antigravity => {
                "agy keeps its Google sign-in in one shared keychain item, so extra accounts use an API key or Application Default Credentials instead."
            }
        }
    }

    pub(crate) fn suggested_env_keys(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"],
            Self::Codex => &["OPENAI_API_KEY", "OPENAI_BASE_URL"],
            Self::Pi => &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY", "OPENROUTER_API_KEY"],
            Self::Antigravity => &[
                "GEMINI_API_KEY",
                "AGY_ADC_AUTH",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_CLOUD_PROJECT",
            ],
        }
    }

    /// Links shared entries from the default dir into `dir`. Existing entries in `dir` are
    /// left alone, and nothing in the default dir is modified.
    pub(crate) fn link_shared_entries(self, dir: &Path) {
        let Some(default_dir) = self.default_config_dir() else {
            return;
        };
        for entry in self.shared_entries() {
            let source = default_dir.join(entry);
            let target = dir.join(entry);
            if !source.exists() || target.symlink_metadata().is_ok() {
                continue;
            }
            #[cfg(unix)]
            if let Err(e) = std::os::unix::fs::symlink(&source, &target) {
                eprintln!("[credentials] link {entry} failed: {e}");
            }
        }
    }

    /// Reads the sign-in state from files only; no CLI is spawned and no keychain is read.
    pub(crate) fn account_status(self, dir: &Path) -> AccountStatus {
        match self {
            Self::Claude => {
                let account = read_json(&dir.join(".claude.json"))
                    .and_then(|v| v.get("oauthAccount").cloned())
                    .filter(Value::is_object);
                AccountStatus {
                    signed_in: account.is_some(),
                    identity: account.and_then(|a| {
                        a.get("emailAddress")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    }),
                }
            }
            Self::Codex => {
                let Some(auth) = read_json(&dir.join("auth.json")) else {
                    return AccountStatus::default();
                };
                let email = auth
                    .pointer("/tokens/id_token")
                    .and_then(Value::as_str)
                    .and_then(jwt_email);
                let api_key = auth
                    .get("OPENAI_API_KEY")
                    .and_then(Value::as_str)
                    .is_some_and(|k| !k.is_empty());
                AccountStatus {
                    signed_in: email.is_some() || api_key,
                    identity: email.or_else(|| api_key.then(|| "API key".to_string())),
                }
            }
            Self::Pi => {
                let providers = read_json(&dir.join("auth.json"))
                    .and_then(|v| v.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()))
                    .unwrap_or_default();
                AccountStatus {
                    signed_in: !providers.is_empty(),
                    identity: (!providers.is_empty()).then(|| providers.join(", ")),
                }
            }
            Self::Antigravity => AccountStatus::default(),
        }
    }

    pub(crate) fn default_account_status(self) -> AccountStatus {
        match self {
            // Without CLAUDE_CONFIG_DIR, Claude keeps `.claude.json` in $HOME, not ~/.claude.
            Self::Claude => {
                let Some(home) = dirs::home_dir() else {
                    return AccountStatus::default();
                };
                Self::Claude.account_status(&home)
            }
            Self::Antigravity => AccountStatus {
                signed_in: dirs::home_dir()
                    .is_some_and(|h| h.join(".gemini/jetski-standalone-oauth-token").exists()),
                identity: None,
            },
            _ => self
                .default_config_dir()
                .map(|dir| self.account_status(&dir))
                .unwrap_or_default(),
        }
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn jwt_email(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_runtimes_share_one_provider() {
        assert_eq!(
            CredentialProvider::for_runtime("claude-native"),
            Some(CredentialProvider::Claude)
        );
        assert_eq!(
            CredentialProvider::for_runtime("claude-code"),
            Some(CredentialProvider::Claude)
        );
        assert_eq!(
            CredentialProvider::for_runtime("agy"),
            Some(CredentialProvider::Antigravity)
        );
        assert_eq!(CredentialProvider::for_runtime("mock"), None);
    }

    #[test]
    fn antigravity_cannot_relocate_its_login() {
        assert_eq!(CredentialProvider::Antigravity.config_dir_env(), None);
        assert_eq!(CredentialProvider::Antigravity.login_command(), None);
    }

    #[test]
    fn reads_codex_email_from_id_token() {
        use base64::Engine;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"email":"dev@example.com"}"#);
        let dir = std::env::temp_dir().join(format!("jockey-cred-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.json"),
            format!(r#"{{"tokens":{{"id_token":"h.{claims}.s"}}}}"#),
        )
        .unwrap();
        let status = CredentialProvider::Codex.account_status(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            status,
            AccountStatus {
                signed_in: true,
                identity: Some("dev@example.com".to_string())
            }
        );
    }
}
