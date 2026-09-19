use crate::db::DbPool;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct AppState {
    pub(crate) db: DbPool,
    /// In-memory cache for role rows; invalidated on upsert/delete.
    /// Wrapped in Arc so temporary AppState clones share the same cache.
    pub(crate) role_cache: Arc<DashMap<String, Arc<Role>>>,
}

impl AppState {
    /// Cheap clone of all shared handles — use when constructing short-lived
    /// temporary AppState instances for spawn_blocking tasks. Every field here must stay
    /// cheap to clone: this runs twice per chat turn, and a plain (non-`Arc`) collection
    /// would be deep-copied on each one.
    pub(crate) fn clone_refs(&self) -> Self {
        Self {
            db: self.db.clone(),
            role_cache: self.role_cache.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Role {
    pub(crate) id: String,
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) runtime_profile_id: String,
    pub(crate) runtime_launch_method: Option<String>,
    pub(crate) system_prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) mcp_servers_json: String,
    pub(crate) config_options_json: String,
    pub(crate) config_option_defs_json: String,
    pub(crate) auto_approve: bool,
    pub(crate) project_id: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Project {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) root_path: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) deleted_at: Option<i64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Workflow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) steps: Vec<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub(crate) id: String,
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
    pub(crate) scope: String,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUpdateEvent {
    pub(crate) session_id: String,
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
    pub(crate) runtime_kind: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) payload: Value,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantRuntime {
    pub(crate) key: String,
    pub(crate) profile_id: String,
    pub(crate) label: String,
    pub(crate) family: String,
    pub(crate) binary: String,
    pub(crate) available: bool,
    pub(crate) version: Option<String>,
    pub(crate) install_hint: Option<String>,
    pub(crate) unavailable_reason: Option<String>,
    pub(crate) launch_method: Option<String>,
    pub(crate) transport: String,
    pub(crate) capabilities: Value,
    pub(crate) input_delivery: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageAttachment {
    pub(crate) data: String,
    pub(crate) mime_type: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatContextOptions {
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) recent_turns: Option<usize>,
    #[serde(default)]
    pub(crate) include_current_role: Option<bool>,
}

impl Default for ChatContextOptions {
    fn default() -> Self {
        Self {
            mode: None,
            recent_turns: None,
            include_current_role: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantChatInput {
    pub(crate) input: String,
    pub(crate) runtime_kind: Option<String>,
    pub(crate) app_session_id: Option<String>,
    #[serde(default)]
    pub(crate) attachments: Vec<ImageAttachment>,
    #[serde(default)]
    pub(crate) context: ChatContextOptions,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleReply {
    pub(crate) role_name: String,
    pub(crate) reply: String,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantChatResponse {
    pub(crate) ok: bool,
    pub(crate) reply: String,
    pub(crate) runtime_kind: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) command_result: Option<ChatCommandResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) role_replies: Vec<RoleReply>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartWorkflowInput {
    pub(crate) workflow_id: String,
    pub(crate) initial_prompt: String,
}

#[derive(Default)]
pub(crate) struct ParsedRouteInput {
    pub(crate) role_names: Vec<String>,
    pub(crate) message: String,
    pub(crate) file_refs: Vec<String>,
    pub(crate) dir_refs: Vec<String>,
    pub(crate) skill_refs: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MentionCandidate {
    pub(crate) value: String,
    pub(crate) kind: String,
    pub(crate) detail: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSession {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) active_role: String,
    pub(crate) runtime_kind: Option<String>,
    pub(crate) runtime_profile_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) external_session_id: Option<String>,
    pub(crate) messages: Vec<serde_json::Value>,
    pub(crate) created_at: i64,
    pub(crate) last_active_at: i64,
    pub(crate) closed_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSessionUpdate {
    pub(crate) title: Option<String>,
    pub(crate) active_role: Option<String>,
    pub(crate) runtime_kind: Option<Option<String>>,
    pub(crate) runtime_profile_id: Option<Option<String>>,
    pub(crate) cwd: Option<Option<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxMessage {
    pub(crate) id: String,
    pub(crate) app_session_id: String,
    pub(crate) role_name: Option<String>,
    pub(crate) delivery: String,
    pub(crate) text: String,
    pub(crate) attachments: Vec<ImageAttachment>,
    pub(crate) created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentLifecycle {
    pub(crate) app_session_id: String,
    pub(crate) role_name: String,
    pub(crate) runtime_kind: String,
    pub(crate) state: String,
    pub(crate) revision: i64,
    pub(crate) last_error: Option<String>,
    pub(crate) updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSkill {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) content: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSkillUpsert {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) content: String,
}

pub(crate) const ATTACH_MAX_TOTAL_BYTES: usize = 160 * 1024;
pub(crate) const ATTACH_MAX_FILE_BYTES: usize = 24 * 1024;
pub(crate) const ATTACH_MAX_DIR_FILES: usize = 30;
pub(crate) const ATTACH_MAX_DIR_DEPTH: usize = 3;
pub(crate) const DEFAULT_MODELS: &[&str] = &[];
pub(crate) const DEFAULT_MCP_SERVERS: &[&str] = &[];
pub(crate) const DEFAULT_SKILLS: &[&str] = &[];
