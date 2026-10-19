import { DEFAULT_ROLE_ALIAS } from "../components/types";
import type { AppSession } from "../components/types";
import type { RawSession } from "./tauriApi";

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

// Auto-titling a session from its first message is engine behaviour
// (`engine::sessions::auto_title`), applied to every client through `turn/started`.

export const makeDefaultSession = (title: string): AppSession => ({
  id: makeSessionId(),
  persisted: true,
  title,
  activeRole: DEFAULT_ROLE_ALIAS,
  runtimeKind: null,
  runtimeProfileId: null,
  cwd: null,
  messages: [],
  streamingMessage: null,
  streamingRunToken: null,
  toolCalls: {},
  streamSegments: [],
  currentPlan: null,
  usage: null,
  notices: [],
  pendingPermissions: [],
  pendingUserInput: [],
  agentModes: [],
  currentMode: null,
  submitting: false,
  turnPhase: "idle",
  discoveredConfigOptions: [],
  configOptionsLoading: false,
  agentCommands: new Map(),
  status: "idle",
  thoughtText: "",
  queuedItems: [],
  previewTabs: [],
  activePreviewTabId: null,
  terminals: {},
  pendingTerminalOutput: {},
  lastError: null,
});

/** A session that exists only in the frontend store, with no `app_sessions` DB row yet.
 *  Opening a project or clicking "+ New Session" should only produce one of these — the row
 *  is created lazily, on the first real send (see `ensureSessionPersisted`), so switching
 *  persona/agent to just look around never spawns anything server-side. */
export const makeDraftSession = (
  title: string,
  opts: {
    projectId?: string | null;
    runtimeKind?: string | null;
    runtimeProfileId?: string | null;
    cwd?: string | null;
    activeRole?: string;
  } = {},
): AppSession => ({
  ...makeDefaultSession(title),
  id: crypto.randomUUID(),
  persisted: false,
  projectId: opts.projectId ?? null,
  runtimeKind: opts.runtimeKind ?? null,
  runtimeProfileId: opts.runtimeProfileId ?? null,
  cwd: opts.cwd ?? null,
  activeRole: opts.activeRole ?? DEFAULT_ROLE_ALIAS,
});

/** A stored session row as the store's `AppSession` (transcript included, live state empty). */
export const sessionFromRaw = (r: RawSession): AppSession => {
  const s = makeDefaultSession(r.title);
  s.id = r.id;
  if (r.activeRole) s.activeRole = r.activeRole;
  if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
  if (r.runtimeProfileId !== undefined) s.runtimeProfileId = r.runtimeProfileId;
  if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
  s.projectId = r.projectId ?? null;
  s.messages = r.messages ?? [];
  return s;
};
