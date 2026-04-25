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

export type AppErrorCode =
  | "VALIDATION_FAILED"
  | "NOT_FOUND"
  | "ALREADY_EXISTS"
  | "DB_ERROR"
  | "PERMISSION_DENIED"
  | "INVALID_INPUT"
  | "ADAPTER_UNAVAILABLE"
  | "UNSUPPORTED_RUNTIME"
  | "INCOMPATIBLE_VERSION"
  | "RATE_LIMITED"
  | "TIMEOUT"
  | "PROCESS_CRASHED"
  | "ACP_ERROR"
  | "FILESYSTEM_ERROR"
  | "INTERNAL_ERROR";

export type AppError = { code: AppErrorCode; message: string };

export function parseError(e: unknown): AppError {
  const raw = String(e);
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && "code" in parsed && "message" in parsed) {
      return parsed as AppError;
    }
  } catch { /* not structured */ }
  return { code: "INTERNAL_ERROR", message: raw };
}

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

export type GlobalMcpEntry = { name: string; configJson: string; isBuiltin: boolean };

export const globalMcpApi = {
  list: () => call<GlobalMcpEntry[]>("list_global_mcp_servers_cmd"),
  upsert: (name: string, configJson: string) =>
    call<void>("upsert_global_mcp_server_cmd", { name, configJson }),
  remove: (name: string) => call<void>("delete_global_mcp_server_cmd", { name }),
};

export type AppRule = {
  id: string;
  name: string;
  content: string;
  description: string | null;
  createdAt: number;
  updatedAt: number;
};

export type RoleRule = {
  ruleId: string;
  name: string;
  content: string;
  description: string | null;
  enabled: boolean;
  ord: number;
};

export const ruleApi = {
  list: () => call<AppRule[]>("list_rules_cmd"),
  upsert: (id: string, name: string, content: string, description?: string | null) =>
    call<void>("upsert_rule_cmd", { id, name, content, description: description ?? null }),
  remove: (id: string) => call<void>("delete_rule_cmd", { id }),
  setRoleRules: (roleName: string, rules: [string, boolean, number][]) =>
    call<void>("set_role_rules_cmd", { roleName, rules }),
  listRoleRules: (roleName: string) => call<RoleRule[]>("list_role_rules_cmd", { roleName }),
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

export type ImageAttachment = { data: string; mimeType: string };

export const assistantApi = {
  chat: (input: { input: string; runtimeKind: string | null; appSessionId: string | null; attachments?: ImageAttachment[] }) =>
    call<AssistantChatResponse>("assistant_chat", { input }),
  detect: () => call<AssistantRuntime[]>("detect_assistants"),
  cancelSession: (roleName: string, appSessionId: string) =>
    call<void>("cancel_acp_session", { roleName, appSessionId }),
  setMode: (roleName: string, modeId: string, appSessionId: string) =>
    call<void>("set_acp_mode", { roleName, modeId, appSessionId }),
  resetSession: (roleName: string, appSessionId: string) =>
    call<void>("reset_acp_session", { roleName, appSessionId }),
  reconnectSession: (roleName: string, appSessionId: string) =>
    call<void>("reconnect_acp_session", { roleName, appSessionId }),
  prewarmRoleConfig: (roleName: string, appSessionId: string) =>
    call<{ configOptions: unknown[]; modes: string[] }>("prewarm_role_config_cmd", { roleName, appSessionId }),
  listDiscoveredConfig: (roleName: string) =>
    call<unknown[]>("list_discovered_config_options_cmd", { roleName }),
  listDiscoveredModes: (roleName: string) =>
    call<string[]>("list_discovered_modes_cmd", { roleName }),
  listAvailableCommands: (roleName: string, appSessionId: string) =>
    call<unknown[]>("list_available_commands_cmd", { roleName, appSessionId }),
  metricsSnapshot: () =>
    call<Array<Record<string, unknown>>>("acp_metrics_snapshot_cmd"),
  logSnapshot: (limit?: number) =>
    call<Array<Record<string, unknown>>>("acp_log_snapshot_cmd", { limit }),
  activeConnections: () =>
    call<Array<Record<string, unknown>>>("active_acp_connections_cmd"),
  syncRoleMode: (roleName: string, modeId: string) =>
    call<string[]>("sync_role_mode_cmd", { roleName, modeId }),
  respondPermission: (requestId: string, optionId: string, cancelled: boolean) =>
    call<void>("respond_permission", { requestId, optionId, cancelled }),
};

export const commandApi = {
  apply: <P extends Record<string, unknown> = Record<string, unknown>>(input: string, appSessionId?: string) =>
    call<Omit<ApplyChatCommandResult, "payload"> & { payload: P }>("apply_chat_command", { input, appSessionId }),
};

export const contextApi = {
  list: (appSessionId: string) =>
    call<Array<{ scope: string; key: string; value: string; updatedAt: number }>>(
      "list_session_context_entries_cmd",
      { appSessionId },
    ),
  set: (appSessionId: string, scope: string, key: string, value: string) =>
    call<{ scope: string; key: string; value: string; updatedAt: number }>(
      "set_session_context_entry_cmd",
      { appSessionId, scope, key, value },
    ),
  remove: (appSessionId: string, scope: string, key: string) =>
    call<void>("delete_session_context_entry_cmd", { appSessionId, scope, key }),
};

export const completionApi = {
  mentions: (query: string, limit: number, appSessionId?: string | null) =>
    call<AppMentionItem[]>("complete_mentions", { query, limit, appSessionId: appSessionId ?? null }),
  cli: (query: string, limit: number) =>
    call<AppMentionItem[]>("complete_cli", { query, limit }),
};

export const configApi = {
  asOptions: (raw: unknown[]) => raw as AcpConfigOption[],
};

export type GitFileEntry = { path: string; statusLetter: string };

export type GitStatus = {
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  detached: boolean;
  staged: GitFileEntry[];
  unstaged: GitFileEntry[];
  untracked: GitFileEntry[];
};

export type GitState =
  | { kind: "not_repo"; cwd: string }
  | { kind: "git_missing" }
  | ({ kind: "status" } & GitStatus);

export type BranchInfo = {
  name: string;
  isCurrent: boolean;
  upstream: string | null;
};

export type GitRemoteInfo = {
  host: string;
  owner: string;
  repo: string;
  webUrl: string;
  branchUrl: string | null;
  prUrl: string | null;
  compareUrl: string | null;
};

export const gitApi = {
  status: (appSessionId?: string | null) =>
    call<GitState>("git_status_cmd", { appSessionId: appSessionId ?? null }),
  listBranches: (appSessionId?: string | null) =>
    call<BranchInfo[]>("git_list_branches_cmd", { appSessionId: appSessionId ?? null }),
  checkout: (appSessionId: string | null | undefined, branch: string) =>
    call<void>("git_checkout_cmd", { appSessionId: appSessionId ?? null, branch }),
  prUrl: (appSessionId?: string | null) =>
    call<string | null>("git_pr_url_cmd", { appSessionId: appSessionId ?? null }),
  remoteInfo: (appSessionId?: string | null) =>
    call<GitRemoteInfo | null>("git_remote_info_cmd", { appSessionId: appSessionId ?? null }),
  diff: (
    appSessionId: string | null | undefined,
    path: string,
    vsHead: boolean,
    staged: boolean,
    untracked: boolean,
  ) =>
    call<string>("git_diff_cmd", {
      appSessionId: appSessionId ?? null,
      path,
      vsHead,
      staged,
      untracked,
    }),
  file: (appSessionId: string | null | undefined, path: string) =>
    call<string>("git_file_cmd", {
      appSessionId: appSessionId ?? null,
      path,
    }),
};

export type DirEntry = { name: string; isDir: boolean };

export const fsApi = {
  listDir: (appSessionId: string | null | undefined, relPath: string, showHidden = false) =>
    call<DirEntry[]>("list_dir_cmd", {
      appSessionId: appSessionId ?? null,
      relPath,
      showHidden,
    }),
  readFileBase64: (appSessionId: string | null | undefined, path: string) =>
    call<string>("read_file_base64_cmd", {
      appSessionId: appSessionId ?? null,
      path,
    }),
};
