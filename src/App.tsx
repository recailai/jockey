import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { createStore, produce } from "solid-js/store";

import SessionTabs from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import ConfigDrawer from "./components/ConfigDrawer";
import type {
  Role, AppToolCall, AppPlanEntry,
  AcpStreamEvent, AcpConfigOption, AssistantRuntime,
  AssistantChatResponse, AcpDeltaEvent, SessionUpdateEvent, WorkflowStateEvent,
  AppMessage, AppMentionItem, AppSkill, AppSession,
} from "./components/types";
import {
  now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS,
  flattenConfigValues,
} from "./components/types";

const ASSISTANT_STORAGE_KEY = "unionai.defaultAssistant";
const MAX_MESSAGES = 500;
const MENTION_DEBOUNCE_MS = 90;
const MENTION_CACHE_LIMIT = 80;

let sessionIdCounter = 0;
const makeSessionId = () => `session-${Date.now()}-${++sessionIdCounter}`;

const makeDefaultSession = (title: string): AppSession => ({
  id: makeSessionId(),
  title,
  teamId: "",
  activeRole: DEFAULT_ROLE_ALIAS,
  selectedAssistant: null,
  messages: [],
  streamingMessage: null,
  toolCalls: new Map(),
  currentPlan: null,
  pendingPermission: null,
  agentModes: [],
  currentMode: null,
  submitting: false,
  discoveredConfigOptions: [],
  configOptionsLoading: false,
  agentCommands: new Map(),
  status: "idle",
});

export default function App() {
  const [sessions, setSessions] = createStore<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  const activeSession = createMemo(() => sessions.find((s) => s.id === activeSessionId()) ?? null);

  const updateSession = (id: string, patch: Partial<AppSession>) => {
    setSessions((s) => s.id === id, produce((s) => Object.assign(s, patch)));
  };

  const patchActiveSession = (patch: Partial<AppSession>) => {
    const id = activeSessionId();
    if (id) updateSession(id, patch);
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
  let streamAccepting = false;
  let streamOriginSessionId: string | null = null;
  let streamBatchBuffer = "";
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
  let persistTimer: number | null = null;
  const HISTORY_MAX = 200;

  const [skills, setSkills] = createSignal<AppSkill[]>([]);

  const scheduleScrollToBottom = () => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const id = activeSessionId();
      const el = id ? listRefMap.get(id) : null;
      if (el) el.scrollTop = el.scrollHeight;
    });
  };

  const persistSessionDebounced = (sessionId: string) => {
    if (persistTimer !== null) window.clearTimeout(persistTimer);
    persistTimer = window.setTimeout(() => {
      persistTimer = null;
      const s = sessions.find((x) => x.id === sessionId);
      if (!s) return;
      void invoke("update_app_session", {
        id: sessionId,
        update: {
          messagesJson: JSON.stringify(s.messages),
          activeRole: s.activeRole,
          selectedAssistant: s.selectedAssistant,
        },
      }).catch(() => {});
    }, 800);
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
    persistSessionDebounced(id);
  };

  const pushMessage = (role: AppMessage["role"], text: string) => {
    appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role, text, at: now() });
  };

  const flushStreamBatch = () => {
    if (streamBatchBuffer) {
      const chunk = streamBatchBuffer;
      streamBatchBuffer = "";
      const sid = streamOriginSessionId ?? activeSessionId();
      if (sid) setSessions((s) => s.id === sid && !!s.streamingMessage, "streamingMessage", "text", (t) => t + chunk);
      scheduleScrollToBottom();
    }
    streamBatchRaf = null;
  };

  const appendStream = (chunk: string) => {
    if (!chunk) return;
    streamBatchBuffer += chunk;
    if (streamBatchRaf === null) {
      streamBatchRaf = window.requestAnimationFrame(flushStreamBatch);
    }
  };

  const resetStreamState = () => {
    if (streamBatchRaf !== null) {
      window.cancelAnimationFrame(streamBatchRaf);
      streamBatchRaf = null;
    }
    streamBatchBuffer = "";
    streamOriginSessionId = null;
  };

  const dropStream = () => {
    patchActiveSession({ streamingMessage: null });
    resetStreamState();
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

  const cancelCurrentRun = async () => {
    if (!activeSession()?.submitting) return;
    canceledRunToken = Math.max(canceledRunToken, runTokenSeq);
    streamAccepting = false;
    dropStream();
    patchActiveSession({ toolCalls: new Map(), currentPlan: null, pendingPermission: null, submitting: false, status: "idle" });
    pushMessage("event", "Cancellation requested.");
    const assistant = activeSession()?.selectedAssistant ?? null;
    const role = activeBackendRole();
    if (assistant) {
      try {
        await invoke("cancel_acp_session", { runtimeKind: assistant, roleName: role, appSessionId: activeSessionId() ?? "" });
      } catch { }
    }
    runNextQueued();
  };

  const refreshRoles = async () => {
    try {
      const rows = await invoke<Role[]>("list_roles", { teamId: null });
      setRoles(rows);
      slashCliCache = null;
    } catch { setRoles([]); }
  };

  const refreshSkills = async () => {
    try {
      const rows = await invoke<AppSkill[]>("list_app_skills");
      setSkills(rows);
    } catch { setSkills([]); }
  };

  const fetchConfigOptions = async (runtimeKey: string): Promise<AcpConfigOption[]> => {
    try {
      const raw = await invoke<unknown[]>("list_discovered_config_options_cmd", { runtimeKey });
      return raw as AcpConfigOption[];
    } catch { return []; }
  };

  const parseAgentCommands = (raw: unknown[]): Array<{ name: string; description: string; hint?: string }> => {
    return (raw as Array<{ name: string; description?: string; input?: { hint?: string } }>).map((c) => ({
      name: c.name, description: c.description ?? "", hint: c.input?.hint,
    }));
  };

  const fetchAndCacheAgentCommands = (runtimeKey: string, roleName: string) => {
    void invoke<unknown[]>("list_available_commands_cmd", { runtimeKey, roleName }).then((raw) => {
      const parsed = parseAgentCommands(raw);
      const sid = activeSessionId();
      if (!sid) return;
      const cidx = sessions.findIndex((s) => s.id === sid);
      if (cidx !== -1) setSessions(cidx, "agentCommands", produce((m: Map<string, Array<{ name: string; description: string; hint?: string }>>) => { m.set(roleName, parsed); }));
    }).catch((e: unknown) => {
      showToast(`Commands unavailable for ${roleName}: ${String(e)}`, "info");
    });
  };

  const setPreferredAssistant = (assistantKey: string | null) => {
    patchActiveSession({ selectedAssistant: assistantKey });
    if (assistantKey) window.localStorage.setItem(ASSISTANT_STORAGE_KEY, assistantKey);
    else window.localStorage.removeItem(ASSISTANT_STORAGE_KEY);
  };

  const refreshAssistants = async () => {
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
    slashCliCache = null;
    const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
    const current = activeSession()?.selectedAssistant ?? null;
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
            selectedTeamId: null,
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

  const buildAgentSlashCandidates = (roleName: string, query: string): AppMentionItem[] => {
    const queryLower = query.toLowerCase().replace(/^\//, "");
    const out: AppMentionItem[] = [];
    const cmds = activeSession()?.agentCommands.get(roleName) ?? [];
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
        if ((s?.discoveredConfigOptions.length ?? 0) === 0) {
          const opts = await fetchConfigOptions(role.runtimeKind);
          if (seq !== slashReqSeq) return;
          patchActiveSession({ discoveredConfigOptions: opts });
        }
        if ((s?.agentCommands.get(role.roleName) ?? []).length === 0) {
          await new Promise<void>((resolve) => {
            void invoke<unknown[]>("list_available_commands_cmd", { runtimeKey: role.runtimeKind, roleName: role.roleName }).then((raw) => {
              const parsed = parseAgentCommands(raw);
              const sid = activeSessionId();
              if (sid) {
                const aidx = sessions.findIndex((sess) => sess.id === sid);
                if (aidx !== -1) setSessions(aidx, "agentCommands", produce((m: Map<string, Array<{ name: string; description: string; hint?: string }>>) => { m.set(role.roleName, parsed); }));
              }
            }).catch(() => {}).finally(() => resolve());
          });
        }
        const candidates = buildAgentSlashCandidates(role.roleName, ctx.query);
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
    let sendRoleLabel = s?.activeRole;
    const isCommand = text.startsWith("/");
    const inRoleContext = isCustomRole();
    const isUnionAiCommand = text.startsWith("/app_");
    const isRoleSlashCmd = isCommand && inRoleContext && !isUnionAiCommand;
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
          routedText = text.replace(/^@\S+\s*/, "").trim();
        } else {
          patchActiveSession({ activeRole: target, discoveredConfigOptions: [] });
          sendRoleLabel = target;
          const targetRole = roles().find((r) => r.roleName === target);
          if (targetRole) {
            void fetchConfigOptions(targetRole.runtimeKind).then((opts) => patchActiveSession({ discoveredConfigOptions: opts }));
            fetchAndCacheAgentCommands(targetRole.runtimeKind, targetRole.roleName);
          }
          routedText = text.replace(/^@\S+\s*/, "").trim();
          if (!routedText) return;
        }
      } else if (isCustomRole() && !text.startsWith("@")) {
        routedText = `@${s?.activeRole ?? ""} ${text}`;
      }
    }

    if (isRoleSlashCmd) {
      routedText = `@${s?.activeRole ?? ""} ${text}`;
    }

    if (!silent) {
      pushMessage("user", text);
      const sid = activeSessionId();
      if (sid) {
        const sess = sessions.find((x) => x.id === sid);
        if (sess && sess.title === "New Session" && sess.messages.filter((m) => m.role === "user").length === 0) {
          const autoTitle = text.slice(0, 30);
          updateSession(sid, { title: autoTitle });
        }
      }
    }
    patchOriginSession({ submitting: true, status: "running", agentState: undefined });

    const isAgentCmd = isCommand && !inRoleContext && /^\/(plan|act|auto|cancel)\b/.test(text);
    if (isAgentCmd) {
      const role = activeBackendRole();
      const assistant = s?.selectedAssistant ?? null;
      const cmd = text.split(/\s+/)[0].slice(1);
      try {
        if (cmd === "cancel") {
          if (assistant) await invoke("cancel_acp_session", { runtimeKind: assistant, roleName: role });
          pushMessage("event", `cancelled ${role}`);
        } else {
          if (assistant) await invoke("set_acp_mode", { runtimeKind: assistant, roleName: role, modeId: cmd });
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
      persistSessionDebounced(originSessionId);
    };

    const startOriginStream = () => {
      streamOriginSessionId = originSessionId;
      const id = `stream-${now()}`;
      const row: AppMessage = { id, role: "assistant", text: "", at: now(), roleLabel: sendRoleLabel };
      patchOriginSession({ streamingMessage: row, toolCalls: new Map(), currentPlan: null, pendingPermission: null });
      scheduleScrollToBottom();
      return row;
    };

    const completeOriginStream = (finalReply?: string) => {
      const sid = originSessionId;
      if (!sid) return;
      const sess = sessions.find((x) => x.id === sid);
      const row = sess?.streamingMessage ?? null;
      const snapshotToolCalls = sess && sess.toolCalls.size > 0 ? [...sess.toolCalls.values()] : undefined;
      if (row) {
        const text = finalReply ?? row.text;
        appendOriginMessage({ ...row, text, at: now(), toolCalls: snapshotToolCalls });
        patchOriginSession({ streamingMessage: null });
      } else if (finalReply) {
        appendOriginMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role: "assistant", text: finalReply, at: now(), roleLabel: sendRoleLabel, toolCalls: snapshotToolCalls });
      }
      resetStreamState();
      patchOriginSession({ toolCalls: new Map(), currentPlan: null, pendingPermission: null });
    };

    const dropOriginStream = () => {
      patchOriginSession({ streamingMessage: null });
      resetStreamState();
    };

    let streamStarted = false;
    if ((!isUnionAiCommand) && s?.selectedAssistant) {
      streamAccepting = true;
      startOriginStream();
      streamStarted = true;
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: {
          input: routedText,
          selectedTeamId: null,
          selectedAssistant: s?.selectedAssistant ?? null,
          appSessionId: originSessionId ?? null,
        }
      });
      if (runToken <= canceledRunToken) return;
      if (res.selectedAssistant) setPreferredAssistant(res.selectedAssistant);

      if (streamStarted) {
        completeOriginStream(res.reply);
      } else {
        appendOriginMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role: "assistant", text: res.reply, at: now(), roleLabel: sendRoleLabel });
      }

    } catch (e) {
      if (runToken <= canceledRunToken) return;
      dropOriginStream();
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) showToast(errMsg);
      setSessions((sess) => sess.id === originSessionId, produce((sess) => {
        sess.messages = [...sess.messages, { id: `${now()}-err`, role: "event" as const, text: errMsg, at: now() }];
      }));
    } finally {
      streamAccepting = false;
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
    if (!activeSession()?.selectedAssistant && !isCustomRole() && !text.startsWith("/app_")) {
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
      s.selectedAssistant = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
    }).catch((e: unknown) => {
      showToast(`Failed to create session: ${String(e)}`);
      const s = makeDefaultSession("New Session");
      s.selectedAssistant = availableAssistant;
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

    void (async () => {
      let loaded: AppSession[] = [];
      try {
        const raw = await invoke<Array<{ id: string; title: string; messagesJson?: string; activeRole?: string; selectedAssistant?: string | null }>>("list_app_sessions");
        loaded = raw.map((r) => {
          const s = makeDefaultSession(r.title);
          s.id = r.id;
          if (r.activeRole) s.activeRole = r.activeRole;
          if (r.selectedAssistant !== undefined) s.selectedAssistant = r.selectedAssistant;
          if (r.messagesJson) {
            try { s.messages = JSON.parse(r.messagesJson) as AppMessage[]; } catch { }
          }
          return s;
        });
      } catch { }

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

      await refreshAssistants();
      await refreshRoles();
      await refreshSkills();

      const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
      const availableAssistant = preferred
        ? assistants().find((a) => a.key === preferred && a.available)?.key ?? null
        : assistants().find((a) => a.available)?.key ?? null;

      sessions.forEach((s, i) => {
        if (!s.selectedAssistant) setSessions(i, "selectedAssistant", availableAssistant);
      });

      pushMessage("system", "Welcome to UnionAI. Agent sessions are warming up in the background.");
    })();

    void Promise.all([
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        if (!streamAccepting) return;
        const sid = ev.payload.appSessionId || streamOriginSessionId || activeSessionId();
        if (!sid) return;
        const sess = sessions.find((s) => s.id === sid);
        if (!sess?.streamingMessage) return;
        appendStream(ev.payload.delta);
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
      listen<{ role: string; appSessionId?: string; event: AcpStreamEvent }>("acp/stream", (ev) => {
        const e = ev.payload.event;
        const sid = ev.payload.appSessionId || streamOriginSessionId || activeSessionId();
        const patchOriginOrActive = (patch: Partial<AppSession>) => { if (sid) updateSession(sid, patch); };
        switch (e.kind) {
          case "statusUpdate":
            if (e.text) patchOriginOrActive({ agentState: e.text });
            break;
          case "thoughtDelta":
            if (e.text) patchOriginOrActive({ agentState: `Thinking: ${e.text.slice(0, 120)}` });
            break;
          case "toolCall":
            if (e.toolCallId && sid) {
              const tidx = sessions.findIndex((s) => s.id === sid);
              if (tidx !== -1) setSessions(tidx, "toolCalls", produce((m: Map<string, AppToolCall>) => {
                m.set(e.toolCallId!, { toolCallId: e.toolCallId!, title: e.title ?? "", kind: e.toolKind ?? "unknown", status: e.status ?? "pending" });
              }));
            }
            break;
          case "toolCallUpdate":
            if (e.toolCallId && sid) {
              const tidx = sessions.findIndex((s) => s.id === sid);
              if (tidx !== -1) setSessions(tidx, "toolCalls", produce((m: Map<string, AppToolCall>) => {
                const existing = m.get(e.toolCallId!);
                if (existing) {
                  const newContent = (e.content as unknown[]) ?? existing.content;
                  const contentJson = newContent !== existing.content
                    ? (newContent && newContent.length > 0 ? JSON.stringify(newContent, null, 2) : undefined)
                    : existing.contentJson;
                  m.set(e.toolCallId!, { ...existing, status: e.status ?? existing.status, title: e.title ?? existing.title, content: newContent, contentJson });
                }
              }));
            }
            break;
          case "plan":
            if (e.entries) patchOriginOrActive({ currentPlan: e.entries as AppPlanEntry[] });
            break;
          case "permissionRequest":
            if (e.requestId) {
              patchOriginOrActive({
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
            if (e.modeId) patchOriginOrActive({ currentMode: e.modeId });
            break;
          case "availableModes":
            if (e.modes) patchOriginOrActive({ agentModes: e.modes });
            if (e.current !== undefined) patchOriginOrActive({ currentMode: e.current ?? null });
            break;
          case "availableCommands":
            if (e.commands && sid) {
              const roleName = ev.payload.role;
              if (roleName) {
                const parsed = parseAgentCommands(e.commands as unknown[]);
                const aidx = sessions.findIndex((s) => s.id === sid);
                if (aidx !== -1) setSessions(aidx, "agentCommands", produce((m: Map<string, Array<{ name: string; description: string; hint?: string }>>) => { m.set(roleName, parsed); }));
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
      window.removeEventListener("keydown", handleGlobalKeyDown);
      dropStream();
      queuedInputs = [];
      if (mentionCloseTimerRef.current !== null) window.clearTimeout(mentionCloseTimerRef.current);
      if (mentionDebounceTimerRef.current !== null) window.clearTimeout(mentionDebounceTimerRef.current);
      if (sessionEventFlushTimer !== null) window.clearTimeout(sessionEventFlushTimer);
      if (persistTimer !== null) window.clearTimeout(persistTimer);
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
        onListMounted={(id, el) => { listRefMap.set(id, el); }}
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
