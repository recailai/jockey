use dashmap::DashMap;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;

pub(crate) struct AppState {
    pub(crate) db: Mutex<Connection>,
    pub(crate) shared_context: DashMap<String, String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Team {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) workspace_path: String,
    pub(crate) created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Role {
    pub(crate) id: String,
    pub(crate) team_id: String,
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) system_prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) mcp_servers_json: String,
    pub(crate) config_options_json: String,
    pub(crate) auto_approve: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Workflow {
    pub(crate) id: String,
    pub(crate) team_id: String,
    pub(crate) name: String,
    pub(crate) steps: Vec<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) team_id: String,
    pub(crate) workflow_id: String,
    pub(crate) status: String,
    pub(crate) initial_prompt: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEvent {
    pub(crate) id: i64,
    pub(crate) session_id: String,
    pub(crate) event_type: String,
    pub(crate) role_name: Option<String>,
    pub(crate) payload: Value,
    pub(crate) created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextEntry {
    pub(crate) team_id: String,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUpdateEvent {
    pub(crate) session_id: String,
    pub(crate) team_id: String,
    pub(crate) workflow_id: String,
    pub(crate) role_name: String,
    pub(crate) delta: String,
    pub(crate) state: String,
    pub(crate) done: bool,
    pub(crate) created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowStateEvent {
    pub(crate) session_id: String,
    pub(crate) team_id: String,
    pub(crate) workflow_id: String,
    pub(crate) status: String,
    pub(crate) active_role: Option<String>,
    pub(crate) message: String,
    pub(crate) created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatCommandResult {
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) selected_team_id: Option<String>,
    pub(crate) selected_assistant: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) payload: Value,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantRuntime {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) binary: String,
    pub(crate) available: bool,
    pub(crate) version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantChatInput {
    pub(crate) input: String,
    pub(crate) selected_team_id: Option<String>,
    pub(crate) selected_assistant: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantChatResponse {
    pub(crate) ok: bool,
    pub(crate) reply: String,
    pub(crate) selected_team_id: Option<String>,
    pub(crate) selected_assistant: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) command_result: Option<ChatCommandResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartWorkflowInput {
    pub(crate) team_id: String,
    pub(crate) workflow_id: String,
    pub(crate) initial_prompt: String,
}

#[derive(Default)]
pub(crate) struct ParsedRouteInput {
    pub(crate) role_names: Vec<String>,
    pub(crate) message: String,
    pub(crate) file_refs: Vec<String>,
    pub(crate) dir_refs: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MentionCandidate {
    pub(crate) value: String,
    pub(crate) kind: String,
    pub(crate) detail: String,
}

pub(crate) const ATTACH_MAX_TOTAL_BYTES: usize = 160 * 1024;
pub(crate) const ATTACH_MAX_FILE_BYTES: usize = 24 * 1024;
pub(crate) const ATTACH_MAX_DIR_FILES: usize = 40;
pub(crate) const ATTACH_MAX_DIR_DEPTH: usize = 6;
pub(crate) const DEFAULT_WORKSPACE_NAME: &str = "default";
pub(crate) const PREWARM_ROLE_LIMIT: usize = 6;
pub(crate) const DEFAULT_MODELS: &[&str] = &[];
pub(crate) const KNOWN_RUNTIME_KEYS: &[&str] = &["gemini-cli", "claude-code", "codex-cli", "mock"];
pub(crate) const DEFAULT_MCP_SERVERS: &[&str] = &[];
pub(crate) const DEFAULT_SKILLS: &[&str] = &[];
pub(crate) const BASE_CLI_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show command help"),
    ("/assistant list", "List detected assistant runtimes"),
    (
        "/assistant select <runtime>",
        "Select active assistant runtime",
    ),
    ("/model list", "List configurable model catalog"),
    ("/model add <model>", "Add model to dynamic catalog"),
    ("/model remove <model>", "Remove model from dynamic catalog"),
    ("/model select <model>", "Select assistant model"),
    (
        "/model select role <role> <model>",
        "Select model for a specific role",
    ),
    ("/model get", "Get selected assistant model"),
    ("/model clear", "Clear selected assistant model"),
    ("/mcp list", "List MCP catalog and enabled entries"),
    ("/mcp add <name>", "Add MCP server to catalog"),
    ("/mcp remove <name>", "Remove MCP server from catalog"),
    ("/mcp enable <name>", "Enable MCP server"),
    ("/mcp disable <name>", "Disable MCP server"),
    ("/skill list", "List skill catalog and enabled entries"),
    ("/skill add <name>", "Add skill to catalog"),
    ("/skill remove <name>", "Remove skill from catalog"),
    ("/skill enable <name>", "Enable skill"),
    ("/skill disable <name>", "Disable skill"),
    ("/role list", "List roles"),
    (
        "/role bind <role> <runtime> [prompt]",
        "Create or update role",
    ),
    ("/role prompt <role> <prompt>", "Update role system prompt"),
    ("/role delete <role>", "Delete role"),
    ("/role edit <role> model <model>", "Set role model"),
    ("/role edit <role> mode <mode>", "Set role mode"),
    (
        "/role edit <role> auto-approve <true|false>",
        "Set role auto-approve",
    ),
    ("/role edit <role> mcp-add <json>", "Add MCP server to role"),
    (
        "/role edit <role> mcp-remove <name>",
        "Remove MCP server from role",
    ),
    ("/role copy <src> <dst>", "Duplicate role"),
    ("/workflow list", "List workflows"),
    ("/workflow create <name> <r1,r2>", "Create workflow"),
    ("/workflow start <name> <prompt>", "Start workflow"),
    ("/session list", "List sessions"),
    ("/session stop <id>", "Stop session"),
    ("/session reset assistant", "Reset assistant session"),
    ("/session reset role <name>", "Reset role session"),
    ("/context list", "List assistant context"),
    ("/context list role <name>", "List role context"),
    ("/context set <key> <value>", "Set assistant context"),
    (
        "/context set role <name> <key> <value>",
        "Set role context value",
    ),
    ("/context get <key>", "Get assistant context value"),
    ("/context delete <key>", "Delete assistant context value"),
    ("/run <prompt>", "Run quick workflow"),
];
