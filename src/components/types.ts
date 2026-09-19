export type Project = {
  id: string;
  name: string;
  rootPath: string;
  createdAt: number;
  updatedAt: number;
};

export type Role = {
  id: string; roleName: string; runtimeKind: string; runtimeProfileId?: string; runtimeLaunchMethod?: string | null;
  systemPrompt: string; model: string | null; mode: string | null;
  mcpServersJson: string; configOptionsJson: string; configOptionDefsJson: string; autoApprove: boolean;
  projectId?: string | null;
};
export type RoleUpsertInput = {
  id?: string;
  roleName: string; runtimeKind: string; runtimeProfileId?: string; systemPrompt: string;
  model: string | null; mode: string | null; mcpServersJson: string; configOptionsJson: string;
  configOptionDefsJson?: string | null;
  autoApprove: boolean;
  projectId?: string | null;
};
export type AppToolCall = {
  toolCallId: string;
  title: string;
  /** Provider-native tool identifier, when the protocol exposes one. */
  toolName?: string;
  kind: string;
  status: string;
  content?: unknown[];
  contentJson?: string;
  locations?: Array<{ path: string; line?: number }>;
  rawInput?: unknown;
  rawOutput?: unknown;
  rawInputJson?: string;
  rawOutputJson?: string;
  terminalMeta?: unknown;
  /** Owning tool call when the provider reports nesting (sub-agents, spawned tasks). */
  parentId?: string | null;
  /** Structured patch for file-editing tools. */
  diff?: unknown;
  /** Accumulated streamed stdout for long-running tools. */
  outputLog?: string;
  /** Agent that owns this call when a turn fans out across roles. */
  roleName?: string;
};
export type AppPlanEntry = { content?: string; title?: string; status?: string; description?: string; priority?: string };
/** Token accounting for the session. `null` fields mean the provider did not report them,
 *  which is different from reporting zero. */
export type AppUsage = {
  inputTokens: number | null;
  outputTokens: number | null;
  cacheReadTokens: number | null;
  cacheWriteTokens: number | null;
  reasoningTokens: number | null;
  totalTokens: number | null;
  contextWindow: number | null;
  costUsd: number | null;
};
export type AppNotice = { id: string; level: string; code: string | null; text: string; at: number };
export type AppUserInputOption = { value: string; label: string; description?: string | null };
export type AppUserInputQuestion = {
  id: string;
  header?: string | null;
  prompt: string;
  options: AppUserInputOption[];
  allowOther: boolean;
  secret: boolean;
  multi: boolean;
};
/** A structured question set from the agent. Unlike a permission this is not allow/deny. */
export type AppUserInputRequest = {
  requestId: string;
  title: string | null;
  blocking: boolean;
  questions: AppUserInputQuestion[];
};
export type AppPermission = { requestId: string; title: string; description: string | null; options: Array<{ optionId: string; title?: string; kind?: string }> };
export type AgentBlock =
  | { kind: "text"; text: string; roleName?: string }
  | { kind: "thought"; text: string; channel?: "raw" | "summary"; roleName?: string }
  | { kind: "tool"; tc: AppToolCall; roleName?: string }
  | { kind: "image"; data: string; mimeType: string; roleName?: string }
  | { kind: "error"; code: string; message: string; roleName?: string }
  | { kind: "other"; type: string; payload: unknown; roleName?: string };
export type AgentEventEnvelope = {
  schemaVersion: number;
  sessionId: string;
  turnId: string;
  roleName: string;
  runtimeKey: string;
  seq: number;
  event: AcpStreamEvent;
};
export type AcpStreamEvent = {
  kind: string;
  text?: string;
  toolCallId?: string; title?: string; toolName?: string; toolKind?: string; status?: string; content?: unknown[];
  locations?: Array<{ path: string; line?: number }>;
  rawInput?: unknown;
  rawOutput?: unknown;
  terminalMeta?: unknown;
  entries?: AppPlanEntry[];
  requestId?: string; description?: string | null; options?: unknown[];
  // `userInputRequest` event fields.
  questions?: AppUserInputQuestion[]; blocking?: boolean;
  // `toolCallUpdate` nesting / diff, and `toolOutputDelta`.
  parentId?: string | null; diff?: unknown; delta?: string;
  modeId?: string;
  commands?: unknown[];
  modes?: Array<{ id: string; title?: string }>; current?: string | null;
  // `sessionError` event fields — typed session error surface from the worker.
  code?: string; message?: string; retryable?: boolean;
  // `usage` event fields.
  inputTokens?: number | null; outputTokens?: number | null;
  cacheReadTokens?: number | null; cacheWriteTokens?: number | null;
  reasoningTokens?: number | null; totalTokens?: number | null;
  contextWindow?: number | null; costUsd?: number | null;
  // `notice` event fields (`text` and `code` are shared with other variants).
  level?: string;
  // `contextCompacted` event fields (`reason` shares the shape of other detail fields).
  beforeTokens?: number | null; afterTokens?: number | null; reason?: string | null;
  // Provider event preserved by an adapter when it has no canonical rendering.
  typeName?: string;
  raw?: unknown;
};
export type AcpStreamPayload = {
  role: string;
  runtimeKind?: string;
  appSessionId?: string;
  seq?: number;
  turnId?: string;
  schemaVersion?: number;
  event: AcpStreamEvent;
};

/** Display-only terminal view derived from ToolCall.meta.terminal_* payloads.
 *  One entry per terminal_id; updated in-place as `terminalOutput` chunks arrive. */
export type TerminalEntry = {
  terminalId: string;
  label: string | null;
  cwd: string | null;
  output: string;
  exitStatus: { exitCode?: number; signal?: string | null } | null;
};

export type SessionErrorInfo = {
  code: string;
  message: string;
  retryable: boolean;
};
export type ConfigOptionValue = { value: string; name: string; description?: string };
export type ConfigOptionGroup = { group: string; name: string; options: ConfigOptionValue[] };
export type AcpConfigOption = {
  id: string; name: string; description?: string;
  category?: string;
  type: "select";
  currentValue: string;
  options: ConfigOptionValue[] | ConfigOptionGroup[];
};
export type InputDeliveryCapabilities = {
  nextTurn: boolean;
  nextStep: boolean;
  interrupt: boolean;
  runNow: boolean;
  strategy?: "queue" | "interruptFollowUp" | "steer" | string;
};
export type AssistantRuntime = { key: string; profileId: string; label: string; family: "native" | "acp" | string; binary: string; available: boolean; version: string | null; installHint: string | null; unavailableReason: string | null; launchMethod: string | null; transport: string; capabilities: Record<string, boolean | string>; inputDelivery?: InputDeliveryCapabilities };
export type RuntimeCapabilities = {
  streaming?: boolean;
  sessionPersistence?: boolean;
  sessionResume?: boolean;
  modelCatalog?: boolean;
  dynamicModes?: boolean;
  dynamicConfig?: boolean;
  permissionRequests?: boolean;
  mcpServers?: boolean;
  attachments?: boolean;
  toolInvocations?: boolean;
  sessionListing?: boolean;
  rewind?: boolean;
  fork?: boolean;
  toolDetail?: boolean;
  interaction?: "none" | "permissionOnly" | "full" | string;
  outputStreaming?: boolean;
  usage?: boolean;
  nestedTools?: boolean;
  toolOutputStreaming?: boolean;
  structuredUserInput?: boolean;
  terminal?: boolean;
  plan?: boolean;
};
export type QueuedItem = {
  id: string;
  clientId?: string;
  text: string;
  attachments: Array<{ data: string; mimeType: string }>;
  roleName?: string | null;
  delivery: "nextTurn" | "nextStep";
  createdAt: number;
  status: "queued" | "claimed" | "failed";
};
export type ChatCommandResult = { ok: boolean; message: string; runtimeKind: string | null; sessionId: string | null; payload: Record<string, unknown> };
export type AssistantChatResponse = { ok: boolean; reply: string; runtimeKind: string | null; sessionId: string | null; commandResult: ChatCommandResult | null; roleReplies?: Array<{ roleName: string; reply: string; ok?: boolean; errorCode?: string | null }> };
export type SessionUpdateEvent = { sessionId: string; roleName: string; delta: string; done: boolean };
export type WorkflowStateEvent = { sessionId: string; status: string; activeRole: string | null; message: string };
export type AcpDeltaEvent = {
  role: string;
  delta: string;
  appSessionId?: string;
  turnId?: string;
  schemaVersion?: number;
};
export type AppSegment =
  | { kind: "text"; text: string; roleName?: string }
  | { kind: "thought"; text: string; roleName?: string }
  | { kind: "tool"; tc: AppToolCall; roleName?: string }
  | { kind: "other"; type: string; payload: unknown; roleName?: string };
export type AppMessageType = "user" | "assistant" | "tool" | "thought" | "event" | "context";
export type AppMessage = {
  id: string;
  roleName: string;
  text: string;
  at: number;
  messageType?: AppMessageType;
  purpose?: "request" | "response" | "providerEvent" | "roleHandoff" | "taskState" | "conversationHistory";
  toolCalls?: AppToolCall[];
  segments?: AppSegment[];
  images?: { data: string; mimeType: string }[];
  thoughtText?: string;
};
export type AppMentionItem = {
  value: string;
  kind: "role" | "file" | "dir" | "command" | "skill";
  detail: string;
  /** Where a slash candidate came from, so the menu can label agent vs Jockey commands. */
  source?: "agent" | "jockey" | "client";
};

export type PreviewMode = "diff" | "file" | "preview" | "image" | "commit";
export type PreviewTab = {
  id: string;
  cwd: string;
  path: string;
  label: string;
  initialMode: PreviewMode;
  staged: boolean;
  untracked: boolean;
  commitOid?: string | null;
};
export type AppSkill = { id: string; name: string; description: string; content: string; createdAt: number; updatedAt: number };

export type AppSession = {
  id: string;
  /** false until this session has an `app_sessions` DB row — see `makeDraftSession`.
   *  A draft only exists in the frontend store; switching persona/model on it must not
   *  hit any Tauri command that assumes a real row (FK inserts, session-role bindings). */
  persisted: boolean;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  runtimeProfileId?: string | null;
  cwd: string | null;
  projectId?: string | null;
  messages: AppMessage[];
  streamingMessage: AppMessage | null;
  /** Run token that currently owns `streamingMessage`. Set by `startOriginStream`
   *  and checked by `finalizeSessionStream` so that a late-arriving response from
   *  a cancelled run cannot clobber a newer run's live stream.
   *  Typed as number (not RunToken) to avoid circular imports; runToken.ts is the
   *  authoritative source. null = no run owns it. */
  streamingRunToken: number | null;
  toolCalls: Record<string, AppToolCall>;
  streamSegments: AppSegment[];
  currentPlan: AppPlanEntry[] | null;
  usage: AppUsage | null;
  notices: AppNotice[];
  pendingPermissions: AppPermission[];
  pendingUserInput: AppUserInputRequest[];
  agentModes: Array<{ id: string; title?: string }>;
  currentMode: string | null;
  submitting: boolean;
  turnPhase?: "idle" | "running" | "cancelling" | "sending";
  /** True while the queue runner owns the session, including the provider turn it starts. */
  queueRunActive?: boolean;
  discoveredConfigOptions: AcpConfigOption[];
  configOptionsLoading: boolean;
  agentCommands: Map<string, Array<{ name: string; description: string; hint?: string }>>;
  status: "idle" | "running" | "done" | "error";
  agentState?: string;
  thoughtText?: string;
  queuedItems: QueuedItem[];
  previewTabs: PreviewTab[];
  activePreviewTabId: string | null;
  /** Display-only terminal entries keyed by terminal_id. Populated from
   *  `ToolCall.meta.terminal_info`; `terminal_output` chunks append to
   *  `output`; `terminal_exit` sets `exitStatus`. */
  terminals: Record<string, TerminalEntry>;
  /** Buffers `terminal_output` chunks that arrive before the matching
   *  `terminal_info`. Flushed when the info event registers the terminal. */
  pendingTerminalOutput: Record<string, string[]>;
  /** Latest structured ACP-layer error for this session, for UI banner. */
  lastError: SessionErrorInfo | null;
};

export const RUNTIMES = ["claude-native", "codex-cli", "pi-cli", "antigravity-cli", "claude-code", "mock"];
export const RUNTIME_COLOR: Record<string, string> = {
  "claude-native": "runtime-color-claude",
  "antigravity-cli": "runtime-color-antigravity",
  "claude-code": "runtime-color-claude",
  "codex-cli": "runtime-color-codex",
  "pi-cli": "runtime-color-pi",
  mock: "runtime-color-muted",
};
export const INTERACTIVE_MOTION = "motion-safe:transition-colors motion-safe:transition-transform motion-safe:duration-150 motion-safe:ease-out active:scale-[0.98]";
export const DEFAULT_BACKEND_ROLE = "Developer";
export const DEFAULT_ROLE_ALIAS = "Developer";
export const MESSAGE_RENDER_WINDOW = 280;

export const now = (): number => Date.now();
export const fmt = (ts: number): string =>
  new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });

export function flattenConfigValues(opts: ConfigOptionValue[] | ConfigOptionGroup[]): ConfigOptionValue[] {
  if (!opts || opts.length === 0) return [];
  if ("value" in opts[0]) return opts as ConfigOptionValue[];
  return (opts as ConfigOptionGroup[]).flatMap((g) => g.options);
}

// ─────────────────────────────────────────────────────────────────────────────
// Management Domain types
// ─────────────────────────────────────────────────────────────────────────────

export type StoredSession = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  runtimeProfileId?: string | null;
  cwd: string | null;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
};

export type Workflow = {
  id: string;
  name: string;
  description: string;
  steps: WorkflowStep[];
  createdAt: number;
  updatedAt: number;
  status?: "idle" | "running" | "done" | "error";
};

export type WorkflowStep = {
  roleName: string;
  prompt: string;
  order: number;
};

export type McpServerStdio = {
  name: string;
  command: string;
  args: string[];
  env: Array<{ name: string; value: string }>;
};

export type McpServerHttp = {
  type: "http";
  name: string;
  url: string;
  headers: Array<{ name: string; value: string }>;
};

export type McpServerSse = {
  type: "sse";
  name: string;
  url: string;
  headers: Array<{ name: string; value: string }>;
};

export type AcpMcpServer = McpServerStdio | McpServerHttp | McpServerSse;

export type ContextEntry = { scope: string; key: string; value: string; updatedAt: number };

export type TabId = "sessions" | "workflows" | "roles" | "mcp" | "skills" | "rules" | "agents";

export const TABS: Array<{ id: TabId; label: string; icon: string }> = [
  { id: "agents", label: "Agents", icon: "bot" },
  { id: "sessions", label: "Sessions", icon: "history" },
  { id: "workflows", label: "Automations", icon: "git-branch" },
  { id: "roles", label: "Roles", icon: "users" },
  { id: "mcp", label: "MCP", icon: "layers" },
  { id: "skills", label: "Skills", icon: "zap" },
  { id: "rules", label: "Rules", icon: "file-text" },
];
