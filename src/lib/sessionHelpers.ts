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
export const deriveSessionTitleFromMessage = (text: string, existingTitles: string[]): string => {
  const cleaned = text.replace(/[@#][^\s]*/g, "").replace(/^\/\S+\s*/, "").trim();
  const words = cleaned.split(/\s+/);
  let autoTitle = "";
  for (const w of words) {
    if ((autoTitle + " " + w).trim().length > 40) break;
    autoTitle = (autoTitle + " " + w).trim();
  }
  if (!autoTitle) autoTitle = cleaned.slice(0, 30);
  autoTitle = normalizeSessionTitle(autoTitle);
  if (!autoTitle) autoTitle = `Session_${Date.now()}`;
  return uniqueName(autoTitle, existingTitles);
};

/** Should this session's title be auto-derived from the upcoming user message?
 *  True iff title is still the default placeholder and no user message has been sent yet. */
export const shouldAutoTitleSession = (sess: AppSession): boolean =>
  isDefaultSessionTitle(sess.title) &&
  sess.messages.filter((m) => m.roleName === "user").length === 0;

/** Compute a unique auto-title for a session from the first user message. */
export const computeAutoTitleForSession = (
  sess: AppSession,
  allSessions: readonly AppSession[],
  text: string,
): string => {
  const existing = allSessions.filter((x) => x.id !== sess.id).map((x) => x.title);
  return deriveSessionTitleFromMessage(text, existing);
};

export const makeDefaultSession = (title: string): AppSession => ({
  id: makeSessionId(),
  title,
  activeRole: DEFAULT_ROLE_ALIAS,
  runtimeKind: null,
  cwd: null,
  messages: [],
  streamingMessage: null,
  streamingRunToken: null,
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
  previewTabs: [],
  activePreviewTabId: null,
});
