import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";
import { createStore, produce } from "solid-js/store";

import SessionTabs from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import type {
  Role, AppPlanEntry,
  AcpStreamEvent, AcpConfigOption, AssistantRuntime,
  AssistantChatResponse, AcpDeltaEvent, SessionUpdateEvent, WorkflowStateEvent,
  AppMessage, AppMentionItem, AppSkill, AppSession, AppToolCall,
} from "./components/types";
import {
  now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS,
  flattenConfigValues,
} from "./components/types";

const ConfigDrawer = lazy(() => import("./components/ConfigDrawer"));

const ASSISTANT_STORAGE_KEY = "unionai.defaultAssistant";
const MAX_MESSAGES = 500;
const MAX_THOUGHT_CHARS = 5000;
const MENTION_DEBOUNCE_MS = 90;
const MENTION_CACHE_LIMIT = 80;

let sessionIdCounter = 0;
const makeSessionId = () => `session-${Date.now()}-${++sessionIdCounter}`;

const makeDefaultSession = (title: string): AppSession => ({
  id: makeSessionId(),
  title,
  activeRole: DEFAULT_ROLE_ALIAS,
  runtimeKind: null,
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
});

export default function App() {
  const [sessions, setSessions] = createStore<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  const activeSession = createMemo(() => sessions.find((s) => s.id === activeSessionId()) ?? null);

  const updateSession = (id: string, patch: Partial<AppSession>) => {
    setSessions((s) => s.id === id, produce((s) => Object.assign(s, patch)));
  };

  const persistSessionPatch = (id: string, patch: Partial<AppSession>) => {
    const update: { title?: string; activeRole?: string; runtimeKind?: string | null | undefined } = {};
    if (typeof patch.title === "string") update.title = patch.title;
    if (typeof patch.activeRole === "string") update.activeRole = patch.activeRole;
    if ("runtimeKind" in patch) update.runtimeKind = patch.runtimeKind ?? null;
    if (Object.keys(update).length === 0) return;
    void invoke("update_app_session", { id, update }).catch(() => { });
  };

  const patchActiveSession = (patch: Partial<AppSession>) => {
    const id = activeSessionId();
    if (!id) return;
    if ("runtimeKind" in patch) {
      if (patch.runtimeKind) window.localStorage.setItem(ASSISTANT_STORAGE_KEY, patch.runtimeKind);
      else window.localStorage.removeItem(ASSISTANT_STORAGE_KEY);
    }
    updateSession(id, patch);
    persistSessionPatch(id, patch);
  };

  const [roles, setRoles] = createSignal<Role[]>([]);
  const [assistants, setAssistants] = createSignal<AssistantRuntime[]>([]);
  const [input, setInput] = createSignal("");
  const [mentionOpen, setMentionOpen] = createSignal(false);
  const [mentionItems, setMentionItems] = createSignal<AppMentionItem[]>([]);
  const [mentionActiveIndex, setMentionActiveIndex] = createSignal(0);
  const [mentionRange, setMentionRange] = createSignal<{ start: number; end: number; query: string } | null>(null);
  const [slashOpen, setSlashOpen] = createSignal(false);
  const [slashItems, setSlashItems] = createSignal<AppMentionItem[]>([]);
  const [slashActiveIndex, setSlashActiveIndex] = createSignal(0);
  const [slashRange, setSlashRange] = createSignal<{ end: number; query: string } | null>(null);
  const [showDrawer, setShowDrawer] = createSignal(false);

  type Toast = { id: number; message: string; severity?: "error" | "info" };
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  let toastSeq = 0;
  const showToast = (message: string, severity: Toast["severity"] = "error") => {
    const id = ++toastSeq;
    setToasts((ts) => [...ts, { id, message, severity }]);
    window.setTimeout(() => setToasts((ts) => ts.filter((t) => t.id !== id)), 4000);
  };

  let mentionReqSeq = 0;
  let slashReqSeq = 0;
  const mentionCloseTimerRef = { current: null as number | null };
  const mentionDebounceTimerRef = { current: null as number | null };
  let mentionPathCache = new Map<string, AppMentionItem[]>();
  let mentionPathCacheKeys: string[] = [];
  let slashCliCache: AppMentionItem[] | null = null;
  let slashCliCacheVersion = 0;
  let inputEl: HTMLInputElement | undefined;
  const acceptingStreams = new Set<string>();
  const streamBatchBuffers = new Map<string, string>();
  let streamBatchRaf: number | null = null;
  let runTokenSeq = 0;
  let canceledRunToken = 0;
  let queuedInputs: string[] = [];
  let scrollRaf: number | null = null;
  const listRefMap = new Map<string, HTMLElement>();
  let pendingSessionEvents: string[] = [];
  let sessionEventFlushTimer: number | null = null;
  let inputHistory: string[] = [];
  let historyIndex = -1;
  let historySavedInput = "";
  const HISTORY_MAX = 200;

  const [skills, setSkills] = createSignal<AppSkill[]>([]);
  const normalizeRuntimeKey = (runtimeKey: string): string => {
    const k = runtimeKey.trim().toLowerCase();
    if (k === "claude" || k === "claude-acp") return "claude-code";
    if (k === "gemini") return "gemini-cli";
    if (k === "codex" || k === "codex-acp") return "codex-cli";
    return k;
  };
  const commandCacheKey = (runtimeKey: string, roleName: string) => `${runtimeKey}:${roleName}`;

  const scheduleScrollToBottom = () => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const id = activeSessionId();
      const el = id ? listRefMap.get(id) : null;
      if (el) el.scrollTop = el.scrollHeight;
    });
  };

  createEffect(() => {
    if (!activeSessionId()) return;
    const session = activeSession();
    void session?.messages.length;
    void session?.streamingMessage?.text.length;
    scheduleScrollToBottom();
  });

  const persistMessage = (sessionId: string, message: AppMessage) => {
    if (!sessionId || message.roleName === "event") return;
    void invoke("append_app_message", {
      sessionId,
      roleName: message.roleName,
      content: message.text,
    }).catch(() => {});
  };

  const appendMessage = (message: AppMessage) => {
    const id = activeSessionId();
    if (!id) return;
    const idx = sessions.findIndex((s) => s.id === id);
    if (idx === -1) return;
    setSessions(idx, "messages", produce((msgs: AppMessage[]) => {
      if (msgs.length >= MAX_MESSAGES) msgs.splice(0, msgs.length - MAX_MESSAGES + 1);
      msgs.push(message);
    }));
    scheduleScrollToBottom();
    persistMessage(id, message);
  };

  const pushMessage = (roleName: string, text: string) => {
    appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName, text, at: now() });
  };

  const flushStreamBatch = () => {
    for (const [sid, buf] of streamBatchBuffers) {
      if (!buf) continue;
      const chunk = buf;
      streamBatchBuffers.set(sid, "");
      setSessions(
        (s) => s.id === sid && !!s.streamingMessage,
        produce((s) => {
          s.streamingMessage!.text = (s.streamingMessage!.text ?? "") + chunk;
          const last = s.streamSegments[s.streamSegments.length - 1];
          if (last && last.kind === "text") {
            s.streamSegments[s.streamSegments.length - 1] = { kind: "text" as const, text: last.text + chunk };
          } else {
            s.streamSegments.push({ kind: "text" as const, text: chunk });
          }
        }),
      );
    }
    scheduleScrollToBottom();
    streamBatchRaf = null;
  };

  const appendStream = (sessionId: string, chunk: string) => {
    if (!chunk) return;
    const existing = streamBatchBuffers.get(sessionId) ?? "";
    streamBatchBuffers.set(sessionId, existing + normalizeNewlines(chunk));
    if (streamBatchRaf === null) {
      streamBatchRaf = window.requestAnimationFrame(flushStreamBatch);
    }
  };

  const resetStreamState = (sessionId?: string) => {
    if (streamBatchRaf !== null) {
      window.cancelAnimationFrame(streamBatchRaf);
      streamBatchRaf = null;
    }
    if (sessionId) {
      streamBatchBuffers.delete(sessionId);
      acceptingStreams.delete(sessionId);
    } else {
      streamBatchBuffers.clear();
      acceptingStreams.clear();
    }
  };

  const dropStream = () => {
    patchActiveSession({ streamingMessage: null });
    resetStreamState(activeSessionId() ?? undefined);
  };

  const scheduleSessionEventFlush = () => {
    if (sessionEventFlushTimer !== null) return;
    sessionEventFlushTimer = window.setTimeout(() => {
      sessionEventFlushTimer = null;
      if (pendingSessionEvents.length === 0) return;
      pushMessage("event", pendingSessionEvents.join("\n"));
      pendingSessionEvents = [];
    }, 120);
  };

  const runNextQueued = () => {
    const s = activeSession();
    if (s?.submitting) return;
    const next = queuedInputs.shift();
    if (!next) return;
    void sendRaw(next);
  };

  const isCustomRole = () => {
    const s = activeSession();
    return s ? s.activeRole !== DEFAULT_ROLE_ALIAS && s.activeRole !== DEFAULT_BACKEND_ROLE : false;
  };

  const activeBackendRole = () => isCustomRole() ? (activeSession()?.activeRole ?? DEFAULT_BACKEND_ROLE) : DEFAULT_BACKEND_ROLE;

  const runtimeForRole = (roleName: string): string | null => {
    if (!isCustomRole() || roleName === DEFAULT_BACKEND_ROLE) return activeSession()?.runtimeKind ?? null;
    return roles().find((r) => r.roleName === roleName)?.runtimeKind ?? activeSession()?.runtimeKind ?? null;
  };

  const cancelCurrentRun = async () => {
    const sid = activeSessionId();
    const sess = sid ? sessions.find((s) => s.id === sid) : null;
    if (!sess?.submitting || !sid) return;
    canceledRunToken = Math.max(canceledRunToken, runTokenSeq);
    acceptingStreams.delete(sid);
    updateSession(sid, { streamingMessage: null });
    resetStreamState(sid);
    updateSession(sid, { toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, thoughtText: "", submitting: false, status: "idle" });
    pushMessage("event", "Cancellation requested.");
    const role = activeBackendRole();
    const runtime = runtimeForRole(role);
    if (runtime) {
      try {
        await invoke("cancel_acp_session", { runtimeKind: runtime, roleName: role, appSessionId: sid });
      } catch { }
    }
    runNextQueued();
  };

  const refreshRoles = async () => {
    try {
      const rows = await invoke<Role[]>("list_roles");
      setRoles(rows);
      slashCliCache = null;
    } catch { setRoles([]); }
  };

  createEffect(() => {
    if (!activeSession()) return;
    void refreshRoles();
  });

  const refreshSkills = async () => {
    try {
      const rows = await invoke<AppSkill[]>("list_app_skills");
      setSkills(rows);
    } catch { setSkills([]); }
  };

  const fetchConfigOptions = async (runtimeKey: string, roleName?: string): Promise<AcpConfigOption[]> => {
    try {
      const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
      if (roleName) {
        const raw = await invoke<unknown[]>("prewarm_role_config_cmd", {
          runtimeKind: normalizedRuntime,
          roleName,
        });
        return raw as AcpConfigOption[];
      }
      const cached = await invoke<unknown[]>("list_discovered_config_options_cmd", { runtimeKey: normalizedRuntime });
      if (cached.length > 0) return cached as AcpConfigOption[];
      const raw = await invoke<unknown[]>("prewarm_role_config_cmd", {
        runtimeKind: normalizedRuntime,
        roleName: "",
      });
      return raw as AcpConfigOption[];
    } catch { return []; }
  };

  const parseAgentCommands = (raw: unknown[]): Array<{ name: string; description: string; hint?: string }> => {
    return (raw as Array<{ name: string; description?: string; input?: { hint?: string } }>).map((c) => ({
      name: c.name, description: c.description ?? "", hint: c.input?.hint,
    }));
  };

  const normalizeNewlines = (input: string): string => input.replace(/\r\n?/g, "\n");

  const normalizeToolLocations = (
    raw: unknown[] | undefined,
  ): Array<{ path: string; line?: number }> | undefined => {
    if (!Array.isArray(raw) || raw.length === 0) return undefined;
    const out = raw
      .map((item) => {
        if (!item || typeof item !== "object") return null;
        const obj = item as Record<string, unknown>;
        const path = typeof obj.path === "string" ? obj.path : "";
        if (!path) return null;
        const line = typeof obj.line === "number" ? obj.line : undefined;
        return { path, line };
      })
      .filter((item): item is NonNullable<typeof item> => !!item);
    return out.length > 0 ? out : undefined;
  };

  const appendThought = (sessionId: string, chunk: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized.trim()) return;
    setSessions(
      (s) => s.id === sessionId,
      "thoughtText",
      (prev) => {
        const next = `${prev ?? ""}${normalized}`;
        return next.length <= MAX_THOUGHT_CHARS
          ? next
          : next.slice(next.length - MAX_THOUGHT_CHARS);
      },
    );
  };

  const fetchAgentCommands = async (runtimeKey: string, roleName: string): Promise<{ runtimeKey: string; commands: Array<{ name: string; description: string; hint?: string }> }> => {
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    try {
      const raw = await invoke<unknown[]>("list_available_commands_cmd", { runtimeKey: normalizedRuntime, roleName });
      return { runtimeKey: normalizedRuntime, commands: parseAgentCommands(raw) };
    } catch {
      return { runtimeKey: normalizedRuntime, commands: [] };
    }
  };

  const hydrateAgentCommandsForSession = async (
    sessionId: string,
    runtimeKey: string,
    roleName: string,
  ): Promise<number> => {
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    const result = await fetchAgentCommands(normalizedRuntime, roleName);

    const aidx = sessions.findIndex((sess) => sess.id === sessionId);
    const commandKey = commandCacheKey(result.runtimeKey, roleName);
    if (aidx !== -1) {
      setSessions(aidx, "agentCommands", (m) => {
        const next = new Map(m);
        next.set(commandKey, result.commands);
        return next;
      });
    }
    return result.commands.length;
  };

  const fetchAndCacheAgentCommands = (runtimeKey: string, roleName: string) => {
    void (async () => {
      const sid = activeSessionId();
      if (!sid) return;
      await hydrateAgentCommandsForSession(sid, runtimeKey, roleName);
    })().catch((e: unknown) => {
      showToast(`Commands unavailable for ${roleName}: ${String(e)}`, "info");
    });
  };

  const setPreferredAssistant = (assistantKey: string | null) => {
    patchActiveSession({ runtimeKind: assistantKey });
  };

  const refreshAssistants = async () => {
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
    slashCliCache = null;
    const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
    const current = activeSession()?.runtimeKind ?? null;
    const currentAvailable = current ? rows.find((a) => a.key === current && a.available) : null;
    if (currentAvailable) return;
    const preferredAvailable = preferred ? rows.find((a) => a.key === preferred && a.available) : null;
    const first = preferredAvailable ?? rows.find((a) => a.available) ?? null;
    if (first) {
      setPreferredAssistant(first.key);
    }
  };

  const closeMentionMenu = () => {
    setMentionOpen(false);
    setMentionItems([]);
    setMentionActiveIndex(0);
    setMentionRange(null);
  };

  const closeSlashMenu = () => {
    setSlashOpen(false);
    setSlashItems([]);
    setSlashActiveIndex(0);
    setSlashRange(null);
  };

  const extractMentionContext = (text: string, caret: number) => {
    const left = text.slice(0, caret);
    for (const trigger of ["@", "#"]) {
      const at = left.lastIndexOf(trigger);
      if (at < 0) continue;
      const prev = at > 0 ? left[at - 1] : " ";
      if (!/\s/.test(prev)) continue;
      const query = left.slice(at + 1);
      if (/\s/.test(query)) continue;
      let end = caret;
      while (end < text.length && !/\s/.test(text[end])) end += 1;
      return { start: at + 1, end, query, trigger };
    }
    return null;
  };

  const listRoleMentionCandidates = (query: string) => {
    const q = query.startsWith("role:") ? query.slice(5).toLowerCase() : query.toLowerCase();
    const out: AppMentionItem[] = [];
    for (const role of roles()) {
      const nameLower = role.roleName.toLowerCase();
      if (q && !nameLower.includes(q)) continue;
      out.push({ value: role.roleName, kind: "role", detail: role.runtimeKind });
    }
    return out;
  };

  const extractSlashContext = (text: string, caret: number) => {
    const left = text.slice(0, caret);
    if (!left.startsWith("/")) return null;
    if (left.includes("\n")) return null;
    return { end: caret, query: left.trimEnd() };
  };

  const shouldPathComplete = (query: string) => {
    return query.startsWith("file:")
      || query.startsWith("dir:")
      || query.includes("/")
      || query.startsWith(".")
      || query.startsWith("~")
      || query.length === 0;
  };

  const refreshMentionSuggestions = async (text: string, caret: number) => {
    const ctx = extractMentionContext(text, caret);
    if (!ctx) {
      closeMentionMenu();
      return;
    }
    setMentionRange(ctx);
    if (ctx.trigger === "#") {
      const q = ctx.query.toLowerCase();
      const skillItems: AppMentionItem[] = skills()
        .filter((s) => !q || s.name.toLowerCase().includes(q))
        .map((s) => ({ value: s.name, kind: "skill" as const, detail: s.description || s.content.slice(0, 60) }));
      setMentionItems(skillItems);
      setMentionActiveIndex(0);
      if (skillItems.length > 0) setMentionOpen(true); else closeMentionMenu();
      return;
    }
    const staticItems: AppMentionItem[] = [];
    if (ctx.query.length === 0) {
      staticItems.push(
        { value: "file:", kind: "hint", detail: "explicit file path" },
        { value: "dir:", kind: "hint", detail: "explicit directory path" },
      );
    }
    let items = [...staticItems, ...listRoleMentionCandidates(ctx.query)];

    if (shouldPathComplete(ctx.query)) {
      const seq = ++mentionReqSeq;
      const cached = mentionPathCache.get(ctx.query);
      if (cached) {
        items = [...items, ...cached];
      } else {
        try {
          const rows = await invoke<AppMentionItem[]>("complete_mentions", {
            query: ctx.query,
            limit: 12,
          });
          if (seq !== mentionReqSeq) return;
          mentionPathCache.set(ctx.query, rows);
          mentionPathCacheKeys.push(ctx.query);
          if (mentionPathCacheKeys.length > MENTION_CACHE_LIMIT) {
            const evictKey = mentionPathCacheKeys.shift()!;
            mentionPathCache.delete(evictKey);
          }
          items = [...items, ...rows];
        } catch {
        }
      }
    }

    const dedup = new Set<string>();
    const merged = items.filter((it) => {
      const key = `${it.kind}:${it.value}`;
      if (dedup.has(key)) return false;
      dedup.add(key);
      return true;
    }).slice(0, 12);

    if (merged.length === 0) {
      closeMentionMenu();
      return;
    }
    setMentionItems(merged);
    setMentionActiveIndex(0);
    setMentionOpen(true);
  };

  const buildAgentSlashCandidates = (runtimeKey: string, roleName: string, query: string): AppMentionItem[] => {
    const queryLower = query.toLowerCase().replace(/^\//, "");
    const out: AppMentionItem[] = [];
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    const cmds = activeSession()?.agentCommands.get(commandCacheKey(normalizedRuntime, roleName)) ?? [];
    for (const cmd of cmds) {
      const value = `/${cmd.name}`;
      if (queryLower && !cmd.name.toLowerCase().includes(queryLower)) continue;
      out.push({ value, kind: "command", detail: cmd.description });
    }
    for (const opt of activeSession()?.discoveredConfigOptions ?? []) {
      const vals = flattenConfigValues(opt.options);
      for (const v of vals) {
        const value = `/${opt.id} ${v.value}`;
        if (queryLower && !value.toLowerCase().includes(queryLower)) continue;
        out.push({ value, kind: "command", detail: `${opt.name}: ${v.name}` });
      }
    }
    return out.slice(0, 30);
  };

  const refreshSlashSuggestions = async (text: string, caret: number) => {
    const ctx = extractSlashContext(text, caret);
    if (!ctx) {
      closeSlashMenu();
      return;
    }
    setSlashRange(ctx);
    const seq = ++slashReqSeq;

    if (isCustomRole() && !ctx.query.startsWith("/app_")) {
      const s = activeSession();
      const role = roles().find((r) => r.roleName === s?.activeRole);
      if (role) {
        const runtimeKey = normalizeRuntimeKey(role.runtimeKind);
        const key = commandCacheKey(runtimeKey, role.roleName);
        if ((s?.discoveredConfigOptions.length ?? 0) === 0) {
          const opts = await fetchConfigOptions(runtimeKey, role.roleName);
          if (seq !== slashReqSeq) return;
          patchActiveSession({ discoveredConfigOptions: opts });
        }
        if ((s?.agentCommands.get(key) ?? []).length === 0) {
          const sid = activeSessionId();
          if (sid) await hydrateAgentCommandsForSession(sid, runtimeKey, role.roleName);
        }
        const candidates = buildAgentSlashCandidates(runtimeKey, role.roleName, ctx.query);
        if (seq !== slashReqSeq) return;
        if (candidates.length === 0) { closeSlashMenu(); return; }
        setSlashItems(candidates);
        setSlashActiveIndex(0);
        setSlashOpen(true);
        return;
      }
    }

    try {
      const version = slashCliCacheVersion;
      const all = slashCliCache ?? await (async () => {
        const rows = await invoke<AppMentionItem[]>("complete_cli", { query: "", limit: 200 });
        if (slashCliCacheVersion === version) {
          slashCliCache = rows;
        }
        return rows;
      })();
      if (seq !== slashReqSeq) return;
      const q = ctx.query.toLowerCase();
      const filtered = q
        ? all.filter((r) => r.value.toLowerCase().includes(q)).slice(0, 20)
        : all.slice(0, 20);
      if (filtered.length === 0) {
        closeSlashMenu();
        return;
      }
      setSlashItems(filtered);
      setSlashActiveIndex(0);
      setSlashOpen(true);
    } catch {
      closeSlashMenu();
    }
  };

  const refreshInputCompletions = (value: string, caret: number) => {
    if (extractSlashContext(value, caret)) {
      closeMentionMenu();
      void refreshSlashSuggestions(value, caret);
      return;
    }
    closeSlashMenu();
    void refreshMentionSuggestions(value, caret);
  };

  const applyMentionCandidate = (item: AppMentionItem) => {
    const target = inputEl;
    if (!target) return;
    const range = mentionRange();
    if (!range) return;
    const current = input();
    const left = current.slice(0, range.start);
    const right = current.slice(range.end);
    const next = `${left}${item.value} ${right}`;
    setInput(next);
    closeMentionMenu();
    const caret = range.start + item.value.length + 1;
    queueMicrotask(() => {
      target.focus();
      target.setSelectionRange(caret, caret);
    });
  };

  const applySlashCandidate = (item: AppMentionItem) => {
    const target = inputEl;
    if (!target) return;
    const range = slashRange();
    if (!range) return;
    const current = input();
    const right = current.slice(range.end);
    const next = `${item.value} ${right}`;
    setInput(next);
    closeSlashMenu();
    const caret = item.value.length + 1;
    queueMicrotask(() => {
      target.focus();
      target.setSelectionRange(caret, caret);
    });
  };

  const sendRaw = async (text: string, silent = false) => {
    const runToken = ++runTokenSeq;
    const originSessionId = activeSessionId();
    closeMentionMenu();
    closeSlashMenu();

    const patchOriginSession = (patch: Partial<AppSession>) => {
      if (originSessionId) updateSession(originSessionId, patch);
    };

    const s = activeSession();
    let sendRoleLabel = s?.activeRole ?? DEFAULT_ROLE_ALIAS;
    let effectiveRole = s?.activeRole ?? DEFAULT_ROLE_ALIAS;
    const isCommand = text.startsWith("/");
    let inRoleContext = effectiveRole !== DEFAULT_ROLE_ALIAS && effectiveRole !== DEFAULT_BACKEND_ROLE;
    const isUnionAiCommand = text.startsWith("/app_");
    let routedText = text;

    if (!isCommand) {
      const mentionMatch = text.match(/^@(\S+)/);
      if (mentionMatch) {
        const rawTarget = mentionMatch[1];
        const target = rawTarget.startsWith("role:") ? rawTarget.slice(5) : rawTarget;
        if (
          target === "assistant" ||
          target === DEFAULT_ROLE_ALIAS ||
          target === DEFAULT_BACKEND_ROLE
        ) {
          patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS });
          sendRoleLabel = DEFAULT_ROLE_ALIAS;
          effectiveRole = DEFAULT_ROLE_ALIAS;
          inRoleContext = false;
          routedText = text.replace(/^@\S+\s*/, "").trim();
        } else {
          patchActiveSession({ activeRole: target, discoveredConfigOptions: [] });
          sendRoleLabel = target;
          effectiveRole = target;
          inRoleContext = true;
          const targetRole = roles().find((r) => r.roleName === target);
          if (targetRole) {
            void fetchConfigOptions(targetRole.runtimeKind, targetRole.roleName).then((opts) => patchActiveSession({ discoveredConfigOptions: opts }));
            fetchAndCacheAgentCommands(targetRole.runtimeKind, targetRole.roleName);
          }
          routedText = text.replace(/^@\S+\s*/, "").trim();
          if (!routedText) return;
        }
      } else if (isCustomRole() && !text.startsWith("@")) {
        routedText = `@${effectiveRole} ${text}`;
      }
    }

    const isRoleSlashCmd = isCommand && inRoleContext && !isUnionAiCommand;
    if (isRoleSlashCmd) {
      routedText = `@${effectiveRole} ${text}`;
    }

    if (!silent) {
      pushMessage("user", text);
      const sid = activeSessionId();
      if (sid) {
        const sess = sessions.find((x) => x.id === sid);
        if (sess && sess.title === "New Session" && sess.messages.filter((m) => m.roleName === "user").length === 0) {
          const autoTitle = text.slice(0, 30);
          updateSession(sid, { title: autoTitle });
          void invoke("update_app_session", { id: sid, update: { title: autoTitle } }).catch(() => {});
        }
      }
    }
    patchOriginSession({ submitting: true, status: "running", agentState: undefined, thoughtText: "" });

    const isAgentCmd = isCommand && !inRoleContext && /^\/(plan|act|auto|cancel)\b/.test(text);
    if (isAgentCmd) {
      const role = activeBackendRole();
      const runtime = runtimeForRole(role);
      const cmd = text.split(/\s+/)[0].slice(1);
      try {
        if (cmd === "cancel") {
          if (runtime) await invoke("cancel_acp_session", { runtimeKind: runtime, roleName: role, appSessionId: activeSessionId() ?? "" });
          pushMessage("event", `cancelled ${role}`);
        } else {
          if (runtime) await invoke("set_acp_mode", { runtimeKind: runtime, roleName: role, modeId: cmd, appSessionId: activeSessionId() ?? "" });
          pushMessage("event", `${role} mode → ${cmd}`);
        }
      } catch (e) {
        pushMessage("event", String(e));
      } finally {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued();
      }
      return;
    }

    const appendOriginMessage = (msg: AppMessage) => {
      if (!originSessionId) return;
      setSessions((sess) => sess.id === originSessionId, produce((sess) => {
        sess.messages = [...sess.messages.slice(-MAX_MESSAGES + 1), msg];
      }));
      scheduleScrollToBottom();
      persistMessage(originSessionId, msg);
    };

    const startOriginStream = () => {
      if (originSessionId) acceptingStreams.add(originSessionId);
      const id = `stream-${now()}`;
      const row: AppMessage = { id, roleName: sendRoleLabel, text: "", at: now() };
      patchOriginSession({ streamingMessage: row, toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, thoughtText: "" });
      scheduleScrollToBottom();
      return row;
    };

    const completeOriginStream = (finalReply?: string) => {
      const sid = originSessionId;
      if (!sid) return;
      const sess = sessions.find((x) => x.id === sid);
      const row = sess?.streamingMessage ?? null;
      const snapshotToolCalls = sess && Object.keys(sess.toolCalls).length > 0 ? Object.values(sess.toolCalls) : undefined;
      const snapshotSegments = sess && sess.streamSegments.length > 0 ? [...sess.streamSegments] : undefined;
      if (snapshotSegments && finalReply) {
        const last = snapshotSegments[snapshotSegments.length - 1];
        if (last && last.kind === "text") {
          snapshotSegments[snapshotSegments.length - 1] = { kind: "text", text: normalizeNewlines(finalReply) };
        } else {
          snapshotSegments.push({ kind: "text", text: normalizeNewlines(finalReply) });
        }
      }
      if (row) {
        const text = normalizeNewlines(finalReply ?? row.text);
        appendOriginMessage({ ...row, text, at: now(), toolCalls: snapshotToolCalls, segments: snapshotSegments });
        patchOriginSession({ streamingMessage: null, thoughtText: "" });
      } else if (finalReply) {
        appendOriginMessage({
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: sendRoleLabel,
          text: normalizeNewlines(finalReply),
          at: now(),
          toolCalls: snapshotToolCalls,
          segments: snapshotSegments,
        });
      }
      resetStreamState(sid);
      patchOriginSession({ toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, agentState: undefined, thoughtText: "" });
    };

    const dropOriginStream = () => {
      patchOriginSession({ streamingMessage: null, thoughtText: "" });
      resetStreamState(originSessionId ?? undefined);
    };

    let streamStarted = false;
    if ((!isUnionAiCommand) && s?.runtimeKind) {
      startOriginStream();
      streamStarted = true;
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: {
          input: routedText,
          runtimeKind: s?.runtimeKind ?? null,
          appSessionId: originSessionId ?? null,
        }
      });
      if (runToken <= canceledRunToken) return;
      if (res.runtimeKind) setPreferredAssistant(res.runtimeKind);

      if (streamStarted) {
        completeOriginStream(res.reply);
      } else {
        appendOriginMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: sendRoleLabel, text: res.reply, at: now() });
      }

    } catch (e) {
      if (runToken <= canceledRunToken) return;
      dropOriginStream();
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) showToast(errMsg);
      appendOriginMessage({ id: `${now()}-err`, roleName: "event", text: errMsg, at: now() });
      patchOriginSession({ submitting: false, status: "error" });
      if (originSessionId) acceptingStreams.delete(originSessionId);
      runNextQueued();
      return;
    } finally {
      if (originSessionId) acceptingStreams.delete(originSessionId);
      if (runToken <= canceledRunToken) {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued();
        return;
      }
      patchOriginSession({ submitting: false, status: "done" });
      runNextQueued();
    }
  };

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const text = input().trim();
    if (!text) return;
    if (activeSession()?.submitting) {
      queuedInputs.push(text);
      setInput("");
      pushMessage("system", `Queued (${queuedInputs.length}): ${text}`);
      return;
    }
    if (!activeSession()?.runtimeKind && !isCustomRole() && !text.startsWith("/app_")) {
      pushMessage("system", "Select an assistant or a role first.");
      return;
    }
    closeMentionMenu();
    closeSlashMenu();
    inputHistory.unshift(text);
    if (inputHistory.length > HISTORY_MAX) inputHistory.length = HISTORY_MAX;
    historyIndex = -1;
    historySavedInput = "";
    setInput("");
    await sendRaw(text);
  };

  const handleInputEvent = (el: HTMLInputElement) => {
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    setInput(value);
    historyIndex = -1;
    if (mentionDebounceTimerRef.current !== null) window.clearTimeout(mentionDebounceTimerRef.current);
    mentionDebounceTimerRef.current = window.setTimeout(() => {
      mentionDebounceTimerRef.current = null;
      refreshInputCompletions(value, caret);
    }, MENTION_DEBOUNCE_MS);
  };

  const handleInputKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && activeSession()?.submitting) {
      e.preventDefault();
      void cancelCurrentRun();
      return;
    }

    const slash = slashItems();
    if (slashOpen() && slash.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashActiveIndex((i) => (i + 1) % slash.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashActiveIndex((i) => (i - 1 + slash.length) % slash.length);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        applySlashCandidate(slash[slashActiveIndex()] ?? slash[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeSlashMenu();
        return;
      }
    }

    const items = mentionItems();
    if (mentionOpen() && items.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMentionActiveIndex((i) => (i + 1) % items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionActiveIndex((i) => (i - 1 + items.length) % items.length);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        applyMentionCandidate(items[mentionActiveIndex()] ?? items[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeMentionMenu();
        return;
      }
    }

    if (e.key === "ArrowUp" && inputHistory.length > 0) {
      e.preventDefault();
      if (historyIndex === -1) historySavedInput = input();
      if (historyIndex < inputHistory.length - 1) {
        historyIndex++;
        setInput(inputHistory[historyIndex]);
      }
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (historyIndex > 0) {
        historyIndex--;
        setInput(inputHistory[historyIndex]);
      } else if (historyIndex === 0) {
        historyIndex = -1;
        setInput(historySavedInput);
      }
      return;
    }
  };

  const newSession = () => {
    const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
    const availableAssistant = preferred
      ? assistants().find((a) => a.key === preferred && a.available)?.key ?? null
      : assistants().find((a) => a.available)?.key ?? null;
    void invoke<{ id: string }>("create_app_session", { title: "New Session" }).then((created) => {
      const s = makeDefaultSession("New Session");
      s.id = created.id;
      s.runtimeKind = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
      if (availableAssistant) {
        void invoke("update_app_session", {
          id: created.id,
          update: { runtimeKind: availableAssistant },
        }).catch(() => { });
      }
    }).catch((e: unknown) => {
      showToast(`Failed to create session: ${String(e)}`);
      const s = makeDefaultSession("New Session");
      s.runtimeKind = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
    });
  };

  const closeSession = (id: string) => {
    const remaining = sessions.filter((s) => s.id !== id);
    if (remaining.length === 0) { void invoke("delete_app_session", { id }).catch(() => {}); return; }
    setSessions(remaining);
    if (activeSessionId() === id) {
      const remaining = sessions.filter((s) => s.id !== id);
      if (remaining.length > 0) {
        setActiveSessionId(remaining[remaining.length - 1].id);
      }
    }
    void invoke("delete_app_session", { id }).catch(() => {});
  };

  onMount(() => {
    const handlers: UnlistenFn[] = [];
    let startupRaf: number | null = null;
    const boot = async () => {
      let loaded: AppSession[] = [];
      try {
        const raw = await invoke<Array<{ id: string; title: string; messages: AppMessage[]; activeRole?: string; runtimeKind?: string | null }>>("list_app_sessions");
        loaded = raw.map((r) => {
          const s = makeDefaultSession(r.title);
          s.id = r.id;
          if (r.activeRole) s.activeRole = r.activeRole;
          if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
          s.messages = r.messages ?? [];
          return s;
        });
      } catch (e) {
        showToast(`Failed to restore sessions: ${String(e)}`);
      }

      if (loaded.length === 0) {
        try {
          const created = await invoke<{ id: string }>("create_app_session", { title: "Session 1" });
          const s = makeDefaultSession("Session 1");
          s.id = created.id;
          loaded = [s];
        } catch {
          loaded = [makeDefaultSession("Session 1")];
        }
      }

      setSessions(loaded);
      setActiveSessionId(loaded[0].id);

      await Promise.all([
        refreshAssistants(),
        refreshRoles(),
        refreshSkills(),
      ]);

      const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
      const availableAssistant = preferred
        ? assistants().find((a) => a.key === preferred && a.available)?.key ?? null
        : assistants().find((a) => a.available)?.key ?? null;

      sessions.forEach((s, i) => {
        if (!s.runtimeKind && availableAssistant) {
          setSessions(i, "runtimeKind", availableAssistant);
          void invoke("update_app_session", {
            id: s.id,
            update: { runtimeKind: availableAssistant },
          }).catch(() => { });
        }
      });

      pushMessage("system", "Welcome to UnionAI. Agent sessions are warming up in the background.");
    };
    startupRaf = window.requestAnimationFrame(() => {
      startupRaf = null;
      void boot();
    });

    void Promise.all([
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        const sid = ev.payload.appSessionId;
        if (!sid || !acceptingStreams.has(sid)) return;
        const sess = sessions.find((s) => s.id === sid);
        if (!sess?.streamingMessage) return;
        appendStream(sid, ev.payload.delta);
      }),
      listen<SessionUpdateEvent>("session/update", (ev) => {
        if (!ev.payload.delta) return;
        pendingSessionEvents.push(`[${ev.payload.roleName}] ${ev.payload.delta}`);
        scheduleSessionEventFlush();
      }),
      listen<WorkflowStateEvent>("workflow/state_changed", (ev) => {
        const p = ev.payload;
        pushMessage("event", `[workflow] ${p.status} ${p.activeRole ?? ""} ${p.message}`);
      }),
      listen<{ role: string; runtimeKind?: string; appSessionId?: string; event: AcpStreamEvent }>("acp/stream", (ev) => {
        const e = ev.payload.event;
        const sid = ev.payload.appSessionId;
        if (!sid) return;
        const patchSession = (patch: Partial<AppSession>) => updateSession(sid, patch);
        switch (e.kind) {
          case "statusUpdate":
            if (e.text) patchSession({ agentState: e.text });
            break;
          case "thoughtDelta":
            if (e.text) {
              patchSession({ agentState: `Thinking: ${e.text.slice(0, 120)}` });
              if (sid) appendThought(sid, e.text);
            }
            break;
          case "toolCall":
            if (e.toolCallId) {
              const content = Array.isArray(e.content) ? e.content : undefined;
              const locations = normalizeToolLocations(e.locations as unknown[] | undefined);
              const tc: AppToolCall = {
                toolCallId: e.toolCallId!,
                title: e.title ?? "",
                kind: e.toolKind ?? "unknown",
                status: e.status ?? "pending",
                content,
                contentJson: content && content.length > 0 ? JSON.stringify(content, null, 2) : undefined,
                locations,
                rawInput: e.rawInput,
                rawOutput: e.rawOutput,
              };
              setSessions((s) => s.id === sid, produce((s) => {
                s.toolCalls[e.toolCallId!] = tc;
                s.streamSegments.push({ kind: "tool" as const, tc });
              }));
              patchSession({ agentState: `${e.toolKind ?? "tool"}: ${e.title ?? e.toolCallId}` });
            }
            break;
          case "toolCallUpdate":
            if (e.toolCallId) {
              setSessions((s) => s.id === sid, produce((s) => {
                const existing = s.toolCalls[e.toolCallId!];
                if (!existing) return;
                const newContent = Array.isArray(e.content) ? e.content : existing.content;
                const newLocations = normalizeToolLocations(e.locations as unknown[] | undefined) ?? existing.locations;
                const contentJson = newContent !== existing.content
                  ? (newContent && newContent.length > 0 ? JSON.stringify(newContent, null, 2) : undefined)
                  : existing.contentJson;
                const updated: AppToolCall = {
                  ...existing,
                  kind: e.toolKind ?? existing.kind,
                  status: e.status ?? existing.status,
                  title: e.title ?? existing.title,
                  content: newContent,
                  contentJson,
                  locations: newLocations,
                  rawInput: e.rawInput !== undefined ? e.rawInput : existing.rawInput,
                  rawOutput: e.rawOutput !== undefined ? e.rawOutput : existing.rawOutput,
                };
                s.toolCalls[e.toolCallId!] = updated;
                for (let i = s.streamSegments.length - 1; i >= 0; i--) {
                  const seg = s.streamSegments[i];
                  if (seg.kind === "tool" && seg.tc.toolCallId === e.toolCallId) {
                    s.streamSegments[i] = { kind: "tool", tc: updated };
                    break;
                  }
                }
              }));
              if (e.status || e.title) {
                patchSession({
                  agentState: `${e.toolKind ?? "tool"} ${e.status ?? "updated"}: ${e.title ?? e.toolCallId}`,
                });
              }
            }
            break;
          case "plan":
            if (e.entries) patchSession({ currentPlan: e.entries as AppPlanEntry[] });
            break;
          case "permissionRequest":
            if (e.requestId) {
              patchSession({
                pendingPermission: {
                  requestId: e.requestId,
                  title: e.title ?? "Permission Required",
                  description: e.description ?? null,
                  options: (e.options as Array<{ optionId: string; title?: string }>) ?? [],
                },
              });
            }
            break;
          case "modeUpdate":
            if (e.modeId) patchSession({ currentMode: e.modeId });
            break;
          case "availableModes":
            if (e.modes) patchSession({ agentModes: e.modes });
            if (e.current !== undefined) patchSession({ currentMode: e.current ?? null });
            break;
          case "availableCommands":
            if (e.commands) {
              const roleName = ev.payload.role;
              if (roleName) {
                const parsed = parseAgentCommands(e.commands as unknown[]);
                const runtimeKey = ev.payload.runtimeKind
                  || roles().find((r) => r.roleName === roleName)?.runtimeKind
                  || "";
                const normalizedRuntime = runtimeKey ? normalizeRuntimeKey(runtimeKey) : "";
                if (!normalizedRuntime) break;
                const key = commandCacheKey(normalizedRuntime, roleName);
                setSessions((s) => s.id === sid, produce((s) => {
                  const next = new Map(s.agentCommands);
                  next.set(key, parsed);
                  s.agentCommands = next;
                }));
              }
            }
            break;
          default:
            break;
        }
      }),
    ]).then((hs) => handlers.push(...hs));

    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setShowDrawer((v) => !v);
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);

    onCleanup(() => {
      if (startupRaf !== null) {
        window.cancelAnimationFrame(startupRaf);
        startupRaf = null;
      }
      window.removeEventListener("keydown", handleGlobalKeyDown);
      dropStream();
      queuedInputs = [];
      if (mentionCloseTimerRef.current !== null) window.clearTimeout(mentionCloseTimerRef.current);
      if (mentionDebounceTimerRef.current !== null) window.clearTimeout(mentionDebounceTimerRef.current);
      if (sessionEventFlushTimer !== null) window.clearTimeout(sessionEventFlushTimer);
      if (scrollRaf !== null) {
        window.cancelAnimationFrame(scrollRaf);
        scrollRaf = null;
      }
      closeMentionMenu();
      closeSlashMenu();
      handlers.forEach((h) => h());
    });
  });

  return (
    <div class="window-bg h-dvh overflow-hidden text-[var(--ui-text)] relative flex flex-col">
      <SessionTabs
        sessions={sessions}
        activeSessionId={activeSessionId}
        setActiveSessionId={setActiveSessionId}
        activeSession={activeSession}
        assistants={assistants}
        patchActiveSession={patchActiveSession}
        activeBackendRole={activeBackendRole}
        onNewSession={newSession}
        onCloseSession={closeSession}
        updateSession={updateSession}
        onRefresh={() => { void refreshAssistants(); void refreshRoles(); void refreshSkills(); }}
        onToggleDrawer={() => setShowDrawer((v) => !v)}
      />

      <MessageWindow
        activeSessionId={activeSessionId}
        activeSession={activeSession}
        patchActiveSession={patchActiveSession}
        onListMounted={(id, el) => { listRefMap.set(id, el); scheduleScrollToBottom(); }}
        onListUnmounted={(id) => { listRefMap.delete(id); }}
      />

      <ChatInput
        input={input}
        setInput={setInput}
        activeSession={activeSession}
        patchActiveSession={patchActiveSession}
        isCustomRole={isCustomRole}
        onSubmit={handleSend}
        onInputEvent={handleInputEvent}
        onInputKeyDown={handleInputKeyDown}
        refreshInputCompletions={refreshInputCompletions}
        mentionOpen={mentionOpen}
        mentionItems={mentionItems}
        mentionActiveIndex={mentionActiveIndex}
        slashOpen={slashOpen}
        slashItems={slashItems}
        slashActiveIndex={slashActiveIndex}
        applyMentionCandidate={applyMentionCandidate}
        applySlashCandidate={applySlashCandidate}
        closeMentionMenu={closeMentionMenu}
        closeSlashMenu={closeSlashMenu}
        inputElRef={(el) => { inputEl = el; }}
        mentionCloseTimerRef={mentionCloseTimerRef}
        mentionDebounceTimerRef={mentionDebounceTimerRef}
      />

      <Show when={showDrawer()}>
        <Suspense fallback={null}>
          <ConfigDrawer
            showDrawer={showDrawer}
            setShowDrawer={setShowDrawer}
            assistants={assistants}
            roles={roles}
            skills={skills}
            activeSession={activeSession}
            patchActiveSession={patchActiveSession}
            sendRaw={sendRaw}
            refreshRoles={refreshRoles}
            refreshSkills={refreshSkills}
            pushMessage={pushMessage}
            fetchConfigOptions={fetchConfigOptions}
          />
        </Suspense>
      </Show>

      <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 pointer-events-none">
        <For each={toasts()}>
          {(t) => (
            <div class={`pointer-events-auto max-w-xs rounded-lg px-4 py-2 text-xs shadow-lg ${t.severity === "info" ? "bg-zinc-800 text-zinc-300" : "bg-rose-900/90 text-rose-200"}`}>
              {t.message}
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
