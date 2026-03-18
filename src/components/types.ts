export type Role = {
  id: string; roleName: string; runtimeKind: string;
  systemPrompt: string; model: string | null; mode: string | null;
  mcpServersJson: string; configOptionsJson: string; autoApprove: boolean;
};
export type RoleUpsertInput = {
  roleName: string; runtimeKind: string; systemPrompt: string;
  model: string | null; mode: string | null; mcpServersJson: string; configOptionsJson: string;
  autoApprove: boolean;
};
export type AppToolCall = { toolCallId: string; title: string; kind: string; status: string; content?: unknown[]; contentJson?: string };
export type AppPlanEntry = { title?: string; status?: string; description?: string };
export type AppPermission = { requestId: string; title: string; description: string | null; options: Array<{ optionId: string; title?: string }> };
export type AcpStreamEvent = {
  kind: string;
  text?: string;
  toolCallId?: string; title?: string; toolKind?: string; status?: string; content?: unknown[];
  entries?: AppPlanEntry[];
  requestId?: string; description?: string | null; options?: unknown[];
  modeId?: string;
  commands?: unknown[];
  modes?: Array<{ id: string; title?: string }>; current?: string | null;
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
export type AssistantRuntime = { key: string; label: string; binary: string; available: boolean; version: string | null };
export type ChatCommandResult = { ok: boolean; message: string; selectedAssistant: string | null; sessionId: string | null; payload: Record<string, unknown> };
export type AssistantChatResponse = { ok: boolean; reply: string; selectedAssistant: string | null; sessionId: string | null; commandResult: ChatCommandResult | null };
export type SessionUpdateEvent = { sessionId: string; roleName: string; delta: string; done: boolean };
export type WorkflowStateEvent = { sessionId: string; status: string; activeRole: string | null; message: string };
export type AcpDeltaEvent = { role: string; delta: string; appSessionId?: string };
export type AppMessage = { id: string; role: "system" | "user" | "assistant" | "event"; text: string; at: number; roleLabel?: string; toolCalls?: AppToolCall[] };
export type AppMentionItem = { value: string; kind: "role" | "file" | "dir" | "hint" | "command" | "skill"; detail: string };
export type AppSkill = { id: string; name: string; description: string; content: string; createdAt: number; updatedAt: number };

export type AppSession = {
  id: string;
  title: string;
  activeRole: string;
  selectedAssistant: string | null;
  messages: AppMessage[];
  streamingMessage: AppMessage | null;
  toolCalls: Map<string, AppToolCall>;
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
};

export const RUNTIMES = ["gemini-cli", "claude-code", "codex-cli", "mock"];
export const RUNTIME_COLOR: Record<string, string> = {
  "gemini-cli": "text-blue-300",
  "claude-code": "text-orange-300",
  "codex-cli": "text-purple-300",
  mock: "text-zinc-400",
};
export const INTERACTIVE_MOTION = "motion-safe:transition-colors motion-safe:transition-transform motion-safe:duration-150 motion-safe:ease-out active:scale-[0.98]";
export const DEFAULT_BACKEND_ROLE = "UnionAIAssistant";
export const DEFAULT_ROLE_ALIAS = "UnionAI";
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
