import { invoke } from "@tauri-apps/api/core";
import type {
  AcpConfigOption,
  AppMentionItem,
  AppMessage,
  AppSkill,
  AssistantChatResponse,
  AssistantRuntime,
  Role,
  RoleUpsertInput,
} from "../components/types";

type SessionUpdate = {
  title?: string;
  activeRole?: string;
  runtimeKind?: string | null;
};

type ApplyChatCommandResult = {
  ok: boolean;
  message: string;
  runtimeKind: string | null;
  sessionId: string | null;
  payload: Record<string, unknown>;
};

type RawSession = {
  id: string;
  title: string;
  activeRole?: string;
  runtimeKind?: string | null;
  cwd?: string | null;
  messages?: AppMessage[];
  createdAt?: number;
  lastActiveAt?: number;
  closedAt?: number | null;
};

const call = <T>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);

export const appSessionApi = {
  create: (title: string) => call<{ id: string }>("create_app_session", { title }),
  update: (id: string, update: SessionUpdate) => call<void>("update_app_session", { id, update }),
  remove: (id: string) => call<void>("delete_app_session", { id }),
  list: () => call<RawSession[]>("list_app_sessions"),
  listClosed: () => call<RawSession[]>("list_closed_app_sessions"),
  reopen: (id: string) => call("reopen_app_session", { id }),
  appendMessage: (sessionId: string, roleName: string, content: string) =>
    call<void>("append_app_message", { sessionId, roleName, content }),
};

export const roleApi = {
  list: () => call<Role[]>("list_roles"),
  upsert: (input: RoleUpsertInput) => call<Role>("upsert_role_cmd", { input }),
  remove: (roleName: string) => call<void>("delete_role_cmd", { roleName }),
};

export const skillApi = {
  list: () => call<AppSkill[]>("list_app_skills"),
  upsert: (input: { id?: string; name: string; description: string; content: string }) =>
    call("upsert_app_skill", { input }),
  remove: (id: string) => call<void>("delete_app_skill", { id }),
};

export const workflowApi = {
  list: <T = unknown>() => call<T>("list_workflows"),
  create: (name: string, description: string, steps: unknown[]) =>
    call("create_workflow", { name, description, steps }),
  remove: (id: string) => call<void>("delete_workflow", { id }),
};

export const assistantApi = {
  chat: (input: { input: string; runtimeKind: string | null; appSessionId: string | null }) =>
    call<AssistantChatResponse>("assistant_chat", { input }),
  detect: () => call<AssistantRuntime[]>("detect_assistants"),
  cancelSession: (runtimeKind: string, roleName: string, appSessionId: string) =>
    call<void>("cancel_acp_session", { runtimeKind, roleName, appSessionId }),
  setMode: (runtimeKind: string, roleName: string, modeId: string, appSessionId: string) =>
    call<void>("set_acp_mode", { runtimeKind, roleName, modeId, appSessionId }),
  resetSession: (runtimeKind: string, roleName: string, appSessionId: string) =>
    call<void>("reset_acp_session", { runtimeKind, roleName, appSessionId }),
  prewarmRoleConfig: (runtimeKind: string, roleName: string, appSessionId: string) =>
    call<unknown[]>("prewarm_role_config_cmd", { runtimeKind, roleName, appSessionId }),
  listDiscoveredConfig: (runtimeKey: string, roleName: string, appSessionId: string) =>
    call<unknown[]>("list_discovered_config_options_cmd", { runtimeKey, roleName, appSessionId }),
  listAvailableCommands: (runtimeKey: string, roleName: string, appSessionId: string) =>
    call<unknown[]>("list_available_commands_cmd", { runtimeKey, roleName, appSessionId }),
  respondPermission: (requestId: string, optionId: string, cancelled: boolean) =>
    call<void>("respond_permission", { requestId, optionId, cancelled }),
};

export const commandApi = {
  apply: <P extends Record<string, unknown> = Record<string, unknown>>(input: string, appSessionId?: string) =>
    call<Omit<ApplyChatCommandResult, "payload"> & { payload: P }>("apply_chat_command", { input, appSessionId }),
};

export const completionApi = {
  mentions: (query: string, limit: number) =>
    call<AppMentionItem[]>("complete_mentions", { query, limit }),
  cli: (query: string, limit: number) =>
    call<AppMentionItem[]>("complete_cli", { query, limit }),
};

export const configApi = {
  asOptions: (raw: unknown[]) => raw as AcpConfigOption[],
};
