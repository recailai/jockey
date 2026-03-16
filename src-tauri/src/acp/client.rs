use agent_client_protocol::{self as acp};
use dashmap::DashMap;
use serde_json::json;
use std::sync::OnceLock;

use super::worker::{AcpEvent, DeltaSlot, permission_requests};

struct TerminalHandle {
    child: tokio::sync::Mutex<tokio::process::Child>,
    #[allow(dead_code)]
    output: tokio::sync::Mutex<String>,
}

static TERMINAL_HANDLES: OnceLock<DashMap<String, TerminalHandle>> = OnceLock::new();
fn terminal_handles() -> &'static DashMap<String, TerminalHandle> {
    TERMINAL_HANDLES.get_or_init(DashMap::new)
}

pub(super) struct UnionAiClient {
    pub(super) delta_slot: DeltaSlot,
    pub(super) auto_approve: bool,
}

#[async_trait::async_trait(?Send)]
impl acp::Client for UnionAiClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        if self.auto_approve {
            let outcome = if let Some(opt) = args.options.first() {
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    opt.option_id.clone(),
                ))
            } else {
                acp::RequestPermissionOutcome::Cancelled
            };
            return Ok(acp::RequestPermissionResponse::new(outcome));
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let event = AcpEvent::PermissionRequest {
            request_id: request_id.clone(),
            title: args.tool_call.fields.title.clone().unwrap_or_default(),
            description: None,
            options: args.options.iter().map(|o| serde_json::to_value(o).unwrap_or(json!({}))).collect(),
        };
        if let Ok(guard) = self.delta_slot.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(event);
            }
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        permission_requests().insert(request_id.clone(), tx);
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(decision)) => Ok(acp::RequestPermissionResponse::new(decision)),
            _ => {
                permission_requests().remove(&request_id);
                Ok(acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Cancelled))
            }
        }
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        let event = match args.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                let text = match chunk.content {
                    acp::ContentBlock::Text(tc) => tc.text,
                    acp::ContentBlock::ResourceLink(rl) => rl.uri,
                    _ => return Ok(()),
                };
                if text.is_empty() { return Ok(()); }
                AcpEvent::TextDelta { text }
            }
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = match chunk.content {
                    acp::ContentBlock::Text(tc) => tc.text,
                    _ => return Ok(()),
                };
                if text.is_empty() { return Ok(()); }
                AcpEvent::ThoughtDelta { text }
            }
            acp::SessionUpdate::ToolCall(tc) => {
                AcpEvent::ToolCall {
                    tool_call_id: tc.tool_call_id.to_string(),
                    title: tc.title.clone(),
                    tool_kind: serde_json::to_value(&tc.kind).and_then(|v| Ok(v.as_str().unwrap_or("unknown").to_string())).unwrap_or_else(|_| "unknown".to_string()),
                    status: serde_json::to_value(&tc.status).and_then(|v| Ok(v.as_str().unwrap_or("pending").to_string())).unwrap_or_else(|_| "pending".to_string()),
                }
            }
            acp::SessionUpdate::ToolCallUpdate(tcu) => {
                AcpEvent::ToolCallUpdate {
                    tool_call_id: tcu.tool_call_id.to_string(),
                    status: tcu.fields.status.map(|s| serde_json::to_value(&s).and_then(|v| Ok(v.as_str().unwrap_or("").to_string())).unwrap_or_default()),
                    title: tcu.fields.title.clone(),
                    content: tcu.fields.content.as_ref().map(|blocks| blocks.iter().map(|b| serde_json::to_value(b).unwrap_or(json!({}))).collect()),
                }
            }
            acp::SessionUpdate::Plan(plan) => {
                AcpEvent::Plan {
                    entries: plan.entries.iter().map(|e| serde_json::to_value(e).unwrap_or(json!({}))).collect(),
                }
            }
            acp::SessionUpdate::CurrentModeUpdate(mode) => {
                AcpEvent::ModeUpdate { mode_id: mode.current_mode_id.to_string() }
            }
            acp::SessionUpdate::ConfigOptionUpdate(cfg) => {
                AcpEvent::ConfigUpdate { options: cfg.config_options.iter().map(|o| serde_json::to_value(o).unwrap_or(json!({}))).collect() }
            }
            acp::SessionUpdate::SessionInfoUpdate(info) => {
                AcpEvent::SessionInfo { title: match info.title { acp::MaybeUndefined::Value(v) => Some(v), _ => None } }
            }
            acp::SessionUpdate::AvailableCommandsUpdate(cmds) => {
                AcpEvent::AvailableCommands { commands: cmds.available_commands.iter().map(|c| serde_json::to_value(c).unwrap_or(json!({}))).collect() }
            }
            _ => return Ok(()),
        };
        if let Ok(guard) = self.delta_slot.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(event);
            }
        }
        Ok(())
    }

    async fn read_text_file(&self, args: acp::ReadTextFileRequest) -> acp::Result<acp::ReadTextFileResponse> {
        let path = args.path;
        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        Ok(acp::ReadTextFileResponse::new(content))
    }

    async fn write_text_file(&self, args: acp::WriteTextFileRequest) -> acp::Result<acp::WriteTextFileResponse> {
        if let Some(parent) = args.path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        tokio::fs::write(&args.path, &args.content).await
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        Ok(acp::WriteTextFileResponse::new())
    }

    async fn create_terminal(&self, args: acp::CreateTerminalRequest) -> acp::Result<acp::CreateTerminalResponse> {
        let mut cmd = tokio::process::Command::new(&args.command);
        cmd.args(&args.args);
        if let Some(cwd) = &args.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped())
           .stdin(std::process::Stdio::null());
        let child = cmd.spawn()
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        let terminal_id = acp::TerminalId::from(uuid::Uuid::new_v4().to_string());
        terminal_handles().insert(terminal_id.to_string(), TerminalHandle { child: tokio::sync::Mutex::new(child), output: tokio::sync::Mutex::new(String::new()) });
        Ok(acp::CreateTerminalResponse::new(terminal_id))
    }

    async fn terminal_output(&self, args: acp::TerminalOutputRequest) -> acp::Result<acp::TerminalOutputResponse> {
        let key = args.terminal_id.to_string();
        let handle = terminal_handles().get(&key)
            .ok_or_else(|| acp::Error::new(acp::ErrorCode::InvalidParams.into(), "terminal not found"))?;
        let mut child = handle.child.lock().await;
        let mut output = String::new();
        if let Some(stdout) = child.stdout.as_mut() {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 4096];
            match tokio::time::timeout(std::time::Duration::from_millis(100), stdout.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => output.push_str(&String::from_utf8_lossy(&buf[..n])),
                _ => {}
            }
        }
        let exit_status = child.try_wait().ok().flatten().map(|s| {
            acp::TerminalExitStatus::new().exit_code(s.code().map(|c| c as u32))
        });
        Ok(acp::TerminalOutputResponse::new(output, false).exit_status(exit_status))
    }

    async fn wait_for_terminal_exit(&self, args: acp::WaitForTerminalExitRequest) -> acp::Result<acp::WaitForTerminalExitResponse> {
        let key = args.terminal_id.to_string();
        let handle = terminal_handles().get(&key)
            .ok_or_else(|| acp::Error::new(acp::ErrorCode::InvalidParams.into(), "terminal not found"))?;
        let mut child = handle.child.lock().await;
        let status = child.wait().await
            .map_err(|e| acp::Error::new(acp::ErrorCode::InternalError.into(), e.to_string()))?;
        Ok(acp::WaitForTerminalExitResponse::new(acp::TerminalExitStatus::new().exit_code(status.code().map(|c| c as u32))))
    }

    async fn kill_terminal(&self, args: acp::KillTerminalRequest) -> acp::Result<acp::KillTerminalResponse> {
        let key = args.terminal_id.to_string();
        if let Some(handle) = terminal_handles().get(&key) {
            let mut child = handle.child.lock().await;
            let _ = child.kill().await;
        }
        Ok(acp::KillTerminalResponse::new())
    }

    async fn release_terminal(&self, args: acp::ReleaseTerminalRequest) -> acp::Result<acp::ReleaseTerminalResponse> {
        let key = args.terminal_id.to_string();
        if let Some((_, handle)) = terminal_handles().remove(&key) {
            let mut child = handle.child.lock().await;
            let _ = child.kill().await;
        }
        Ok(acp::ReleaseTerminalResponse::new())
    }
}
