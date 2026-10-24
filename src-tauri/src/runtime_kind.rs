use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeKind {
    Mock,
    ClaudeCode,
    AntigravityCli,
    CodexCli,
    PiCli,
}

impl RuntimeKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mock" => Some(Self::Mock),
            "claude" | "native:claude" | "claude-native" | "acp:claude-legacy" => {
                Some(Self::ClaudeCode)
            }
            "claude-code" | "claude-acp" | "acp:claude-code" => Some(Self::ClaudeCode),
            "agy" | "native:agy" | "acp:agy" | "antigravity" | "antigravity-cli" | "gemini"
            | "gemini-cli" => Some(Self::AntigravityCli),
            "codex" | "codex-cli" | "native:codex" | "acp:codex" => Some(Self::CodexCli),
            "pi" | "pi-cli" | "native:pi" | "acp:pi" => Some(Self::PiCli),
            _ => None,
        }
    }

    pub fn runtime_key(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::ClaudeCode => "claude-code",
            Self::AntigravityCli => "antigravity-cli",
            Self::CodexCli => "codex-cli",
            Self::PiCli => "pi-cli",
        }
    }

    pub fn is_mock(self) -> bool {
        matches!(self, Self::Mock)
    }

    pub fn install_hint(self) -> &'static str {
        match self {
            Self::ClaudeCode => "npm install -g @agentclientprotocol/claude-agent-acp@0.94.0",
            Self::AntigravityCli => "Install agy_acp_server.par from the official ACP registry",
            Self::CodexCli => "npm install -g @agentclientprotocol/codex-acp@2.2.2",
            Self::PiCli => "npm install -g pi-acp@0.0.34",
            Self::Mock => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeKind;

    #[test]
    fn profile_ids_resolve_to_their_runtime_keys() {
        assert_eq!(
            RuntimeKind::from_str("native:agy"),
            Some(RuntimeKind::AntigravityCli)
        );
        assert_eq!(
            RuntimeKind::from_str("native:claude"),
            Some(RuntimeKind::ClaudeCode)
        );
        assert_eq!(
            RuntimeKind::from_str("native:codex"),
            Some(RuntimeKind::CodexCli)
        );
        assert_eq!(RuntimeKind::from_str("native:pi"), Some(RuntimeKind::PiCli));
    }
}
