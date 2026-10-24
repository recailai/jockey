//! Continue a Jockey conversation in the agent's own terminal UI.
//!
//! Jockey coordinates agents via ACP, storing provider session handles so the user
//! can resume the conversation in the agent's native CLI terminal interface.

use rusqlite::{params, OptionalExtension};
use tauri::State;

use crate::credentials::{config_dir_env_for, open_in_terminal, shell_quote};
use crate::db::{get_state, with_db};
use crate::runtime_kind::RuntimeKind;
use crate::types::AppState;

struct ProviderConversation {
    role_name: String,
    runtime_kind: String,
    session_id: String,
}

/// The conversation the session's active role is on, else the most recently used one.
fn latest_conversation(
    state: &AppState,
    app_session_id: &str,
    active_role: &str,
) -> Result<Option<ProviderConversation>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT role_name, runtime_kind, acp_session_id FROM (
                 SELECT role_name, runtime_kind, acp_session_id, updated_at
                   FROM app_session_role_runtime_configs
                  WHERE app_session_id = ?1 AND acp_session_id IS NOT NULL AND acp_session_id != ''
                 UNION ALL
                 SELECT role_name, runtime_kind, acp_session_id, 0
                   FROM app_session_roles
                  WHERE app_session_id = ?1 AND acp_session_id IS NOT NULL AND acp_session_id != ''
             )
             ORDER BY role_name = ?2 DESC, updated_at DESC
             LIMIT 1",
            params![app_session_id, active_role],
            |row| {
                Ok(ProviderConversation {
                    role_name: row.get(0)?,
                    runtime_kind: row.get(1)?,
                    session_id: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

fn resume_command(runtime: RuntimeKind, session_id: &str) -> Result<String, String> {
    let id = shell_quote(session_id);
    match runtime {
        RuntimeKind::ClaudeCode => Ok(format!("claude --resume {id}")),
        RuntimeKind::CodexCli => Ok(format!("codex resume {id}")),
        RuntimeKind::PiCli
        | RuntimeKind::AntigravityCli
        | RuntimeKind::DeepSeekHarness
        | RuntimeKind::Mock => Err(format!(
            "{} sessions cannot be resumed in a terminal yet",
            runtime.runtime_key()
        )),
    }
}

/// Opens Terminal in the session's cwd on `claude --resume <id>` (or the Codex equivalent),
/// under the account the role is bound to. Returns the command that was run.
#[tauri::command]
pub(crate) fn open_session_in_terminal_cmd(
    state: State<'_, AppState>,
    app_session_id: String,
) -> Result<String, String> {
    let state = get_state(&state);
    let head = crate::db::app_session::load_session_head(state, &app_session_id)?
        .ok_or("session not found")?;
    let conversation = latest_conversation(state, &app_session_id, &head.active_role)?
        .ok_or("this session has not started an agent conversation yet")?;
    let runtime = RuntimeKind::from_str(&conversation.runtime_kind)
        .ok_or_else(|| format!("unknown runtime: {}", conversation.runtime_kind))?;
    let mut command = resume_command(runtime, &conversation.session_id)?;
    if let Some((var, dir)) = config_dir_env_for(
        state,
        &conversation.role_name,
        head.project_id.as_deref(),
        &conversation.runtime_kind,
    )? {
        command = format!("{var}={} {command}", shell_quote(&dir));
    }
    let cwd = crate::db::app_session::get_app_session_cwd(state, &app_session_id)
        .ok_or("session has no working directory")?;
    let script = format!("cd {} && {command}", shell_quote(&cwd));
    open_in_terminal(&format!("resume \"{}\"", head.title), &script)?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_runtimes_resume_through_the_cli() {
        assert_eq!(
            resume_command(RuntimeKind::ClaudeCode, "4273a6e0").unwrap(),
            "claude --resume '4273a6e0'"
        );
        assert_eq!(
            resume_command(RuntimeKind::CodexCli, "abc").unwrap(),
            "codex resume 'abc'"
        );
        assert!(resume_command(RuntimeKind::PiCli, "x").is_err());
    }
}
