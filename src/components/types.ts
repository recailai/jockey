export type Role = {
  id: string; roleName: string; runtimeKind: string; runtimeLaunchMethod?: string | null;
  systemPrompt: string; model: string | null; mode: string | null;
  mcpServersJson: string; configOptionsJson: string; configOptionDefsJson: string; autoApprove: boolean;
};
export type RoleUpsertInput = {
  roleName: string; runtimeKind: string; systemPrompt: string;
  model: string | null; mode: string | null; mcpServersJson: string; configOptionsJson: string;
  configOptionDefsJson?: string | null;
  autoApprove: boolean;
};
export type AppToolCall = {
  toolCallId: string;
  title: string;
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
};
export type AppPlanEntry = { content?: string; title?: string; status?: string; description?: string; priority?: string };
export type AppPermission = { requestId: string; title: string; description: string | null; options: Array<{ optionId: string; title?: string; kind?: string }> };
export type AcpStreamEvent = {
  kind: string;
  text?: string;
  toolCallId?: string; title?: string; toolKind?: string; status?: string; content?: unknown[];
  locations?: Array<{ path: string; line?: number }>;
  rawInput?: unknown;
  rawOutput?: unknown;
  terminalMeta?: unknown;
  entries?: AppPlanEntry[];
  requestId?: string; description?: string | null; options?: unknown[];
  modeId?: string;
  commands?: unknown[];
  modes?: Array<{ id: string; title?: string }>; current?: string | null;
  // `sessionError` event fields — typed session error surface from the worker.
  code?: string; message?: string; retryable?: boolean;
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
export type AssistantRuntime = { key: string; label: string; binary: string; available: boolean; version: string | null; installHint: string | null };
export type ChatCommandResult = { ok: boolean; message: string; runtimeKind: string | null; sessionId: string | null; payload: Record<string, unknown> };
export type AssistantChatResponse = { ok: boolean; reply: string; runtimeKind: string | null; sessionId: string | null; commandResult: ChatCommandResult | null };
export type SessionUpdateEvent = { sessionId: string; roleName: string; delta: string; done: boolean };
export type WorkflowStateEvent = { sessionId: string; status: string; activeRole: string | null; message: string };
export type AcpDeltaEvent = { role: string; delta: string; appSessionId?: string };
export type AppSegment = { kind: "text"; text: string } | { kind: "tool"; tc: AppToolCall };
export type AppMessage = { id: string; roleName: string; text: string; at: number; toolCalls?: AppToolCall[]; segments?: AppSegment[]; images?: { data: string; mimeType: string }[]; thoughtText?: string };
export type AppMentionItem = { value: string; kind: "role" | "file" | "dir" | "hint" | "command" | "skill"; detail: string };

export type PreviewMode = "diff" | "file" | "preview" | "image";
export type PreviewTab = {
  id: string;
  cwd: string;
  path: string;
  label: string;
  initialMode: PreviewMode;
  staged: boolean;
  untracked: boolean;
};
export type AppSkill = { id: string; name: string; description: string; content: string; createdAt: number; updatedAt: number };

export type AppSession = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  cwd: string | null;
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
  pendingPermission: AppPermission | null;
  agentModes: Array<{ id: string; title?: string }>;
  currentMode: string | null;
  submitting: boolean;
  discoveredConfigOptions: AcpConfigOption[];
  configOptionsLoading: boolean;
  agentCommands: Map<string, Array<{ name: string; description: string; hint?: string }>>;
  status: "idle" | "running" | "done" | "error";
  agentState?: string;
  thoughtText?: string;
  queuedMessages: string[];
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

export const RUNTIMES = ["gemini-cli", "claude-code", "codex-cli", "mock"];
export const RUNTIME_COLOR: Record<string, string> = {
  "gemini-cli": "runtime-color-gemini",
  "claude-code": "runtime-color-claude",
  "codex-cli": "runtime-color-codex",
  mock: "runtime-color-muted",
};
export const INTERACTIVE_MOTION = "motion-safe:transition-colors motion-safe:transition-transform motion-safe:duration-150 motion-safe:ease-out active:scale-[0.98]";
export const DEFAULT_BACKEND_ROLE = "Jockey";
export const DEFAULT_ROLE_ALIAS = "Jockey";
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
