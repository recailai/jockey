//! Reading a CLI's own `--help` for the values it accepts.
//!
//! Every runtime documents its reasoning/thinking levels there, in its own punctuation, and
//! that text ships with the binary that will actually run. Maintaining the same lists by hand
//! in Rust meant they drifted: the Codex list invented a `minimal` level and omitted the real
//! `max` and `ultra`, and Antigravity's real `--effort` axis was removed outright on the
//! mistaken belief that its model ids replaced it.

use std::time::Duration;
use tokio::process::Command;

const HELP_TIMEOUT: Duration = Duration::from_secs(10);

/// The values documented for `flag`, in whichever shape the CLI writes them:
///
/// - `--effort <level>  Effort level for the current session (low, medium, high, xhigh, max)`
/// - `--effort          Reasoning effort for the current CLI session (low|medium|high)`
/// - `--thinking <level>  Set thinking level: off, minimal, low, medium, high, xhigh, max`
pub(in crate::acp) fn parse_levels(help: &str, flag: &str) -> Option<Vec<String>> {
    let flag_at = help.find(flag)?;
    // Values may wrap onto following lines (Claude does), so read past the flag line — but
    // stop at the next option, or the tail of this one absorbs the start of that one and the
    // last real value is lost to the join.
    let mut window = String::new();
    for (index, line) in help[flag_at..].lines().take(4).enumerate() {
        if index > 0 && line.trim_start().starts_with('-') {
            break;
        }
        if index > 0 {
            window.push(' ');
        }
        window.push_str(line);
    }

    let inside_parens = window.find('(').and_then(|open| {
        window[open..]
            .find(')')
            .map(|close| window[open + 1..open + close].to_string())
    });
    let after_colon = || {
        let colon = window[flag.len()..].find(": ")? + flag.len();
        Some(window[colon + 1..].to_string())
    };

    let levels = split_levels(&inside_parens.or_else(after_colon)?);
    if levels.is_empty() {
        None
    } else {
        Some(levels)
    }
}

fn split_levels(raw: &str) -> Vec<String> {
    raw.split([',', '|'])
        .map(|level| level.trim().trim_end_matches('.').to_string())
        .filter(|level| {
            !level.is_empty()
                && level.len() <= 12
                && level.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .collect()
}

pub(in crate::acp) async fn levels_from_help(
    binary: &str,
    env: &[(String, String)],
    cwd: &str,
    flag: &str,
) -> Option<Vec<String>> {
    let mut cmd = Command::new(binary);
    cmd.arg("--help").kill_on_drop(true);
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
    let output = tokio::time::timeout(HELP_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    parse_levels(&text, flag)
}

#[cfg(test)]
mod tests {
    use super::parse_levels;

    /// Verbatim from `claude --help` (2.1.273) — wrapped across two lines, parenthesised,
    /// comma separated.
    #[test]
    fn reads_claude_effort_levels() {
        let help = "  --disallowed-tools <tools...>   Deny a list of tools\n  \
--effort <level>                      Effort level for the current session\n                                        \
(low, medium, high, xhigh, max)\n  --environment <environment_id>        Create a new cloud session";
        assert_eq!(
            parse_levels(help, "--effort").unwrap(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    /// Verbatim from `agy --help` — parenthesised, pipe separated, on one line. Antigravity
    /// really does have this axis, separately from the `-high`/`-low` suffixes on its models.
    #[test]
    fn reads_antigravity_effort_levels() {
        let help = "  --disable-slash-commands        Disable slash command expansion\n  \
--effort                        Reasoning effort for the current CLI session (low|medium|high)\n  \
-i                              Short alias for --prompt-interactive";
        assert_eq!(
            parse_levels(help, "--effort").unwrap(),
            vec!["low", "medium", "high"]
        );
    }

    /// Verbatim from `pi --help` — no parentheses at all, values after a colon.
    #[test]
    fn reads_pi_thinking_levels() {
        let help = "  --tools <names>                Applies to built-in tools\n  \
--thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh, max\n  \
--extension, -e <path>         Load an extension file";
        assert_eq!(
            parse_levels(help, "--thinking").unwrap(),
            vec!["off", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn a_flag_that_documents_no_values_yields_nothing() {
        let help = "  --verbose    Print more output\n  --quiet    Print less";
        assert_eq!(parse_levels(help, "--verbose"), None);
    }
}
