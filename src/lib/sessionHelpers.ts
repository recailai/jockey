import { DEFAULT_ROLE_ALIAS } from "../components/types";
import type { AppSession } from "../components/types";

export const MAX_MESSAGES = 500;
export const MAX_THOUGHT_CHARS = 5000;
export const MENTION_DEBOUNCE_MS = 90;
export const MENTION_CACHE_LIMIT = 80;

let sessionIdCounter = 0;
export const makeSessionId = () => `session-${Date.now()}-${++sessionIdCounter}`;

export const uniqueName = (desired: string, existing: string[]): string => {
  const set = new Set(existing.map((x) => x.toLowerCase()));
  if (!set.has(desired.toLowerCase())) return desired;
  const base = desired.replace(/_copy(\d+)?$/, "");
  let n = 2;
  let candidate = `${base}_copy`;
  while (set.has(candidate.toLowerCase())) candidate = `${base}_copy${n++}`;
  return candidate;
};

export const normalizeSessionTitle = (raw: string): string => raw.trim().replace(/\s+/g, "_");
export const isDefaultSessionTitle = (raw: string): boolean => /^Session_1(?:_copy\d*)?$/.test(raw);

export const makeDefaultSession = (title: string): AppSession => ({
  id: makeSessionId(),
  title,
  activeRole: DEFAULT_ROLE_ALIAS,
  runtimeKind: null,
  cwd: null,
  messages: [],
  streamingMessage: null,
  toolCalls: {},
  streamSegments: [],
  currentPlan: null,
  pendingPermission: null,
  agentModes: [],
  currentMode: null,
  submitting: false,
  discoveredConfigOptions: [],
  configOptionsLoading: false,
  agentCommands: new Map(),
  status: "idle",
  thoughtText: "",
  queuedMessages: [],
});
