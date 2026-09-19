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
  runtimeProfileId?: string | null;
  cwd?: string | null;
};

type ApplyChatCommandResult = {
  ok: boolean;
  message: string;
  runtimeKind: string | null;
  sessionId: string | null;
  payload: Record<string, unknown>;
};

export type Project = {
  id: string;
  name: string;
  rootPath: string;
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
};

export type RawSession = {
  id: string;
  title: string;
  activeRole?: string;
  runtimeKind?: string | null;
  runtimeProfileId?: string | null;
  cwd?: string | null;
  projectId?: string | null;
  externalSessionId?: string | null;
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

export const projectApi = {
  list: () => call<Project[]>("list_projects_cmd"),
  get: (id: string) => call<Project | null>("get_project_cmd", { id }),
  create: (name: string, rootPath: string) => call<Project>("create_project_cmd", { name, rootPath }),
  remove: (id: string) => call<void>("delete_project_cmd", { id }),
  /** What could be imported, without importing it. Backs the import picker. */
  scanImportable: (projectId: string, forceRefresh = false) =>
    call<ImportableSession[]>("scan_importable_sessions_cmd", { projectId, forceRefresh }),
  /** Omit `externalSessionIds` to import every transcript found. */
  importSessions: (projectId: string, externalSessionIds?: string[]) =>
    call<RawSession[]>("import_project_sessions_cmd", { projectId, externalSessionIds }),
};

export type ImportableSession = {
  sourceId: string;
  externalSessionId: string;
  runtimeKind: string;
  agent: string;
  cwd: string;
  title: string;
  turnCount: number;
  createdAt: number;
  lastActiveAt: number;
  imported: boolean;
};

export type InboxMessage = {
  id: string;
  appSessionId: string;
  roleName: string | null;
  delivery: "nextTurn" | "nextStep" | string;
  text: string;
  attachments: ImageAttachment[];
  createdAt: number;
};
export type AgentLifecycle = {
  appSessionId: string;
  roleName: string;
  runtimeKind: string;
  state: "idle" | "prewarming" | "ready" | "running" | "stopping" | "stopped" | "error" | string;
  revision: number;
  lastError: string | null;
  updatedAt: number;
};

export const appSessionApi = {
  create: (
    title: string,
    projectId?: string,
    runtimeKind?: string | null,
    runtimeProfileId?: string | null,
    id?: string,
  ) =>
    call<{ id: string; title: string }>("create_app_session", {
      title,
      projectId,
      runtimeKind,
      runtimeProfileId,
      id,
    }),
  update: (id: string, update: SessionUpdate) => call<void>("update_app_session", { id, update }),
  remove: (id: string) => call<void>("delete_app_session", { id }),
  list: (projectId?: string) => call<RawSession[]>("list_app_sessions", { projectId }),
  listClosed: () => call<RawSession[]>("list_closed_app_sessions"),
  reopen: (id: string) => call<RawSession>("reopen_app_session", { id }),
  appendMessage: (
    sessionId: string,
    roleName: string,
    content: string,
    contentType?: string,
    payload?: string,
  ) => call<void>("append_app_message", { sessionId, roleName, content, contentType, payload }),
  saveMessage: (
    sessionId: string,
    roleName: string,
    content: string,
    contentType?: string,
    payload?: string,
    clientId?: string,
  ) => call<void>("save_app_message", { sessionId, clientId, roleName, content, contentType, payload }),
  listInbox: (sessionId: string) => call<InboxMessage[]>("list_session_inbox_cmd", { sessionId }),
  enqueueInbox: (
    sessionId: string,
    text: string,
    roleName?: string | null,
    delivery: "nextTurn" | "nextStep" = "nextTurn",
    attachments?: ImageAttachment[],
  ) => call<InboxMessage>("enqueue_session_inbox_cmd", { sessionId, text, roleName, delivery, attachments }),
  removeInbox: (id: string) => call<void>("remove_session_inbox_cmd", { id }),
  claimInbox: (ids: string[]) => call<void>("claim_session_inbox_cmd", { ids }),
  restoreInbox: (ids: string[]) => call<void>("restore_session_inbox_cmd", { ids }),
  listLifecycle: (sessionId: string) => call<AgentLifecycle[]>("list_agent_lifecycle_cmd", { appSessionId: sessionId }),
};

export const roleApi = {
  list: (projectId?: string) => call<Role[]>("list_roles", { projectId }),
  upsert: (input: RoleUpsertInput) => call<Role>("upsert_role_cmd", { input }),
  reassignProject: (roleName: string, projectId: string | null, roleId?: string | null) =>
    call<Role>("reassign_role_project_cmd", { roleName, projectId, roleId }),
  remove: (roleName: string, projectId?: string | null, roleId?: string | null) =>
    call<void>("delete_role_cmd", { roleName, projectId, roleId }),
};

export type RuntimeProfile = {
  id: string;
  label: string;
  family: "native" | "acp" | string;
  transport: string;
  runtimeKey: string;
  capabilities: Record<string, boolean>;
  launchSpec?: {
    command: string;
    args: string[];
    envRefs: string[];
    cwdStrategy: string;
  } | null;
  versionRequirement?: string | null;
  updateStrategy: string;
  builtin: boolean;
};

export const runtimeProfileApi = {
  list: () => call<RuntimeProfile[]>("list_runtime_profiles_cmd"),
  upsert: (input: {
    id?: string;
    label: string;
    command: string;
    args?: string[];
    envRefs?: string[];
    cwdStrategy?: string;
  }) => call<RuntimeProfile>("upsert_runtime_profile_cmd", { input }),
  remove: (id: string) => call<void>("delete_runtime_profile_cmd", { id }),
};

export type ProviderSessionSummary = {
  providerSessionId: string;
  title: string | null;
  cwd: string | null;
  preview: string | null;
  updatedAt: string | null;
  createdAt: string | null;
};

/// Provider session administration (list/import/fork/rewind). Native Codex
/// supports the full surface; native Pi supports import only.
export const providerSessionApi = {
  list: (runtimeKind: string, limit?: number, cwd?: string) =>
    call<ProviderSessionSummary[]>("list_provider_sessions_cmd", {
      runtimeKind,
      limit,
      cwd,
    }),
  import: (input: {
    runtimeKind: string;
    roleName: string;
    appSessionId: string;
    providerSessionId: string;
  }) => call<void>("import_provider_session_cmd", input),
  fork: (input: {
    runtimeKind: string;
    roleName: string;
    appSessionId: string;
    cwd?: string;
  }) => call<string>("fork_provider_session_cmd", input),
  rewind: (input: {
    runtimeKind: string;
    roleName: string;
    appSessionId: string;
    numTurns?: number;
  }) => call<void>("rewind_provider_session_cmd", input),
};

export type GlobalMcpEntry = { name: string; configJson: string; isBuiltin: boolean };

export type RoleMcpEntry = {
  mcpServerName: string;
  configJson: string;
  isBuiltin: boolean;
  enabled: boolean;
};

export const globalMcpApi = {
  list: () => call<GlobalMcpEntry[]>("list_global_mcp_servers_cmd"),
  upsert: (name: string, configJson: string) =>
    call<void>("upsert_global_mcp_server_cmd", { name, configJson }),
  remove: (name: string) => call<void>("delete_global_mcp_server_cmd", { name }),
  listRoleMcp: (roleId: string) => call<RoleMcpEntry[]>("list_role_mcp_servers_cmd", { roleId }),
  setRoleMcpEnabled: (roleId: string, mcpServerName: string, enabled: boolean) =>
    call<void>("set_role_mcp_enabled_cmd", { roleId, mcpServerName, enabled }),
  resetRoleMcpSessions: (roleId: string) =>
    call<string[]>("reset_role_mcp_sessions_cmd", { roleId }),
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

export type RoleSkill = {
  skillId: string;
  name: string;
  content: string;
  description: string;
  enabled: boolean;
  ord: number;
};

export const ruleApi = {
  list: () => call<AppRule[]>("list_rules_cmd"),
  upsert: (id: string, name: string, content: string, description?: string | null) =>
    call<void>("upsert_rule_cmd", { id, name, content, description: description ?? null }),
  remove: (id: string) => call<void>("delete_rule_cmd", { id }),
  setRoleRules: (roleId: string, rules: [string, boolean, number][]) =>
    call<void>("set_role_rules_cmd", { roleId, rules }),
  listRoleRules: (roleId: string) => call<RoleRule[]>("list_role_rules_cmd", { roleId }),
  listAllRulesForRole: (roleId: string) => call<RoleRule[]>("list_all_rules_for_role_cmd", { roleId }),
};

export const skillApi = {
  list: () => call<AppSkill[]>("list_app_skills"),
  upsert: (input: { id?: string; name: string; description: string; content: string }) =>
    call("upsert_app_skill", { input }),
  remove: (id: string) => call<void>("delete_app_skill", { id }),
  listAllSkillsForRole: (roleId: string) =>
    call<RoleSkill[]>("list_all_skills_for_role_cmd", { roleId }),
  setRoleSkills: (roleId: string, skills: [string, boolean, number][]) =>
    call<void>("set_role_skills_cmd", { roleId, skills }),
};

export const workflowApi = {
  list: <T = unknown>() => call<T>("list_workflows"),
  create: (name: string, description: string, steps: unknown[]) =>
    call("create_workflow", { name, description, steps }),
  remove: (id: string) => call<void>("delete_workflow", { id }),
};

export type ImageAttachment = { data: string; mimeType: string };
export type ChatContextOptions = {
  mode?: "handoff" | "history" | "none";
  recentTurns?: number;
  includeCurrentRole?: boolean;
};

export const assistantApi = {
  chat: (input: {
    input: string;
    runtimeKind: string | null;
    appSessionId: string | null;
    attachments?: ImageAttachment[];
    context?: ChatContextOptions;
  }) =>
    call<AssistantChatResponse>("assistant_chat", { input }),
  detect: () => call<AssistantRuntime[]>("detect_assistants"),
  cancelSession: (roleName: string, appSessionId: string) =>
    call<void>("cancel_acp_session", { roleName, appSessionId }),
  steerSession: (
    roleName: string,
    appSessionId: string,
    prompt: string,
    attachments: ImageAttachment[] = [],
  ) => call<void>("steer_acp_session", { roleName, appSessionId, prompt, attachments }),
  setMode: (roleName: string, modeId: string, appSessionId: string) =>
    call<void>("set_acp_mode", { roleName, modeId, appSessionId }),
  resetSession: (roleName: string, appSessionId: string) =>
    call<void>("reset_acp_session", { roleName, appSessionId }),
  reconnectSession: (roleName: string, appSessionId: string) =>
    call<void>("reconnect_acp_session", { roleName, appSessionId }),
  prewarmRoleConfig: (
    roleName: string,
    appSessionId: string,
    projectId?: string | null,
    force?: boolean,
    runtimeKind?: string | null,
  ) =>
    call<{ configOptions: unknown[]; modes: string[] }>("prewarm_role_config_cmd", {
      roleName,
      appSessionId,
      projectId,
      force,
      runtimeKind,
    }),
  listAvailableCommands: (roleName: string, appSessionId: string, projectId?: string | null) =>
    call<unknown[]>("list_available_commands_cmd", { roleName, appSessionId, projectId }),
  metricsSnapshot: () =>
    call<Array<Record<string, unknown>>>("acp_metrics_snapshot_cmd"),
  logSnapshot: (limit?: number) =>
    call<Array<Record<string, unknown>>>("acp_log_snapshot_cmd", { limit }),
  activeConnections: () =>
    call<Array<Record<string, unknown>>>("active_acp_connections_cmd"),
  syncRoleMode: (roleId: string, modeId: string) =>
    call<string[]>("sync_role_mode_cmd", { roleId, modeId }),
  respondUserInput: (requestId: string, answers: Record<string, string[]> | null) =>
    call<void>("respond_user_input_cmd", { requestId, answers }),
  respondPermission: (requestId: string, optionId: string, cancelled: boolean) =>
    call<void>("respond_permission", { requestId, optionId, cancelled }),
};

export type ProjectAgentConfig = {
  projectId: string | null;
  roleName: string;
  /** Engine pinned for this persona in this project; null means use the persona's own. */
  runtimeKind: string | null;
  configOptions: Record<string, string>;
};

/**
 * The Agent layer, keyed by (project, persona). A persona is what the user switches
 * between, so its engine and knobs belong to it — keying by runtime made two personas on
 * the same CLI overwrite each other.
 */
export const projectAgentApi = {
  getConfig: (projectId: string | null, roleName: string) =>
    call<ProjectAgentConfig>("get_project_agent_config_cmd", { projectId, roleName }),
  setConfig: (
    projectId: string | null,
    roleName: string,
    configId: string,
    value: string,
    runtimeKind?: string | null,
  ) =>
    call<ProjectAgentConfig>("set_project_agent_config_cmd", {
      projectId,
      roleName,
      configId,
      value,
      runtimeKind,
    }),
  setRuntime: (projectId: string | null, roleName: string, runtimeKind: string) =>
    call<ProjectAgentConfig>("set_project_agent_runtime_cmd", { projectId, roleName, runtimeKind }),
  /** roleName -> pinned engine, for every persona this project has pinned one for. */
  listPins: (projectId: string | null) =>
    call<Record<string, string>>("list_project_agent_pins_cmd", { projectId }),
  bindSessionAgent: (appSessionId: string, roleName: string, runtimeKind: string) =>
    call<string>("bind_session_agent_cmd", { appSessionId, roleName, runtimeKind }),
};

export const commandApi = {
  apply: <P extends Record<string, unknown> = Record<string, unknown>>(input: string, appSessionId?: string) =>
    call<Omit<ApplyChatCommandResult, "payload"> & { payload: P }>("apply_chat_command", { input, appSessionId }),
};

export type TerminalStartResponse = {
  terminalId: string;
  cwd: string;
  shell: string;
  reused: boolean;
};

export const terminalApi = {
  start: (appSessionId: string, forceNew = false) =>
    call<TerminalStartResponse>("start_terminal_session", { appSessionId, forceNew }),
  resize: (terminalId: string, cols: number, rows: number) =>
    call<void>("resize_terminal_session", { terminalId, cols, rows }),
  write: (terminalId: string, data: string) =>
    call<void>("write_terminal_session", { terminalId, data }),
  stop: (terminalId: string) =>
    call<void>("stop_terminal_session", { terminalId }),
};

export type BuiltinWorkspaceTarget =
  | "vscode"
  | "cursor"
  | "antigravity"
  | "finder"
  | "terminal";

export type WorkspaceOpenTarget = BuiltinWorkspaceTarget | `custom:${string}`;

export type WorkspaceAppRef = {
  appName?: string | null;
  bundleId?: string | null;
};

export const workspaceApi = {
  open: (target: WorkspaceOpenTarget, appSessionId?: string | null, app?: WorkspaceAppRef) =>
    call<void>("open_workspace_cmd", {
      target,
      appSessionId: appSessionId ?? null,
      appName: app?.appName ?? null,
      bundleId: app?.bundleId ?? null,
    }),
  getAppIcon: (target: WorkspaceOpenTarget, app?: WorkspaceAppRef) =>
    call<string | null>("get_workspace_app_icon_cmd", {
      target,
      appName: app?.appName ?? null,
      bundleId: app?.bundleId ?? null,
    }),
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

export type GitCreatedPullRequest = {
  url: string;
  number: number;
};

export type GitCommitEntry = {
  oid: string;
  shortOid: string;
  summary: string;
  authorName: string;
  committedAt: number;
};

export type GitCommitFileEntry = {
  path: string;
  statusLetter: string;
  oldPath?: string | null;
};

export type GitCommitDetail = {
  oid: string;
  shortOid: string;
  summary: string;
  authorName: string;
  committedAt: number;
  files: GitCommitFileEntry[];
  additions: number;
  deletions: number;
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
  commit: (appSessionId: string | null | undefined, input: { message: string; includeUnstaged: boolean }) =>
    call<string>("git_commit_cmd", { appSessionId: appSessionId ?? null, input }),
  stage: (appSessionId: string | null | undefined, path: string) =>
    call<void>("git_stage_cmd", { appSessionId: appSessionId ?? null, path }),
  unstage: (appSessionId: string | null | undefined, path: string) =>
    call<void>("git_unstage_cmd", { appSessionId: appSessionId ?? null, path }),
  fetch: (appSessionId?: string | null) =>
    call<void>("git_fetch_cmd", { appSessionId: appSessionId ?? null }),
  pull: (appSessionId?: string | null) =>
    call<void>("git_pull_cmd", { appSessionId: appSessionId ?? null }),
  push: (appSessionId?: string | null) =>
    call<void>("git_push_cmd", { appSessionId: appSessionId ?? null }),
  createPr: (appSessionId: string | null | undefined, input: { title?: string | null; draft: boolean }) =>
    call<GitCreatedPullRequest>("git_create_pr_cmd", { appSessionId: appSessionId ?? null, input }),
  log: (appSessionId?: string | null, limit = 30) =>
    call<GitCommitEntry[]>("git_log_cmd", { appSessionId: appSessionId ?? null, limit }),
  commitDiff: (appSessionId: string | null | undefined, oid: string) =>
    call<string>("git_commit_diff_cmd", { appSessionId: appSessionId ?? null, oid }),
  commitDetail: (appSessionId: string | null | undefined, oid: string) =>
    call<GitCommitDetail>("git_commit_detail_cmd", { appSessionId: appSessionId ?? null, oid }),
  commitFileDiff: (appSessionId: string | null | undefined, oid: string, path: string) =>
    call<string>("git_commit_file_diff_cmd", {
      appSessionId: appSessionId ?? null,
      oid,
      path,
    }),
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
