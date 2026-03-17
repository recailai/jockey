import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";

type Role = {
  id: string; teamId: string; roleName: string; runtimeKind: string;
  systemPrompt: string; model: string | null; mode: string | null;
  mcpServersJson: string; configOptionsJson: string; autoApprove: boolean;
};
type RoleUpsertInput = {
  teamId: string; roleName: string; runtimeKind: string; systemPrompt: string;
  model: string | null; mode: string | null; mcpServersJson: string; configOptionsJson: string;
  autoApprove: boolean;
};
type AppToolCall = { toolCallId: string; title: string; kind: string; status: string; content?: unknown[]; contentJson?: string };
type AppPlanEntry = { title?: string; status?: string; description?: string };
type AppPermission = { requestId: string; title: string; description: string | null; options: Array<{ optionId: string; title?: string }> };
type AcpStreamEvent = {
  kind: string;
  text?: string;
  toolCallId?: string; title?: string; toolKind?: string; status?: string; content?: unknown[];
  entries?: AppPlanEntry[];
  requestId?: string; description?: string | null; options?: unknown[];
  modeId?: string;
  commands?: unknown[];
  modes?: Array<{ id: string; title?: string }>; current?: string | null;
};
type ConfigOptionValue = { value: string; name: string; description?: string };
type ConfigOptionGroup = { group: string; name: string; options: ConfigOptionValue[] };
type AcpConfigOption = {
  id: string; name: string; description?: string;
  category?: string;
  type: "select";
  currentValue: string;
  options: ConfigOptionValue[] | ConfigOptionGroup[];
};
type AssistantRuntime = { key: string; label: string; binary: string; available: boolean; version: string | null };
type ChatCommandResult = { ok: boolean; message: string; selectedTeamId: string | null; selectedAssistant: string | null; sessionId: string | null; payload: Record<string, unknown> };
type AssistantChatResponse = { ok: boolean; reply: string; selectedTeamId: string | null; selectedAssistant: string | null; sessionId: string | null; commandResult: ChatCommandResult | null };
type SessionUpdateEvent = { sessionId: string; teamId: string; roleName: string; delta: string; done: boolean };
type WorkflowStateEvent = { sessionId: string; teamId: string; status: string; activeRole: string | null; message: string };
type AcpDeltaEvent = { role: string; delta: string };
type AppMessage = { id: string; role: "system" | "user" | "assistant" | "event"; text: string; at: number; roleLabel?: string };
type AppMentionItem = { value: string; kind: "role" | "file" | "dir" | "hint" | "command" | "skill"; detail: string };
type AppSkill = { id: string; name: string; description: string; content: string; createdAt: number; updatedAt: number };

type AppSession = {
  id: string;
  title: string;
  teamId: string;
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
};

const now = () => Date.now();
const fmt = (ts: number) =>
  new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });

const RUNTIMES = ["gemini-cli", "claude-code", "codex-cli", "mock"];
const RUNTIME_COLOR: Record<string, string> = {
  "gemini-cli": "text-blue-300",
  "claude-code": "text-orange-300",
  "codex-cli": "text-purple-300",
  mock: "text-zinc-400",
};
const INTERACTIVE_MOTION = "motion-safe:transition-colors motion-safe:transition-transform motion-safe:duration-150 motion-safe:ease-out active:scale-[0.98]";
const ASSISTANT_STORAGE_KEY = "unionai.defaultAssistant";
const DEFAULT_BACKEND_ROLE = "UnionAIAssistant";
const DEFAULT_ROLE_ALIAS = "UnionAI";
const MAX_MESSAGES = 500;
const MESSAGE_RENDER_WINDOW = 280;
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
  const [sessions, setSessions] = createSignal<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  const activeSession = () => sessions().find((s) => s.id === activeSessionId()) ?? null;

  const updateSession = (id: string, patch: Partial<AppSession>) => {
    setSessions((prev) => prev.map((s) => s.id === id ? { ...s, ...patch } : s));
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
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");

  let mentionReqSeq = 0;
  let slashReqSeq = 0;
  let mentionCloseTimer: number | null = null;
  let mentionDebounceTimer: number | null = null;
  let mentionPathCache = new Map<string, AppMentionItem[]>();
  let inputEl: HTMLInputElement | undefined;
  let slashListEl: HTMLDivElement | undefined;
  let mentionListEl: HTMLDivElement | undefined;
  let streamBuffer = "";
  let streamFlushRaf: number | null = null;
  let streamAccepting = false;
  let runTokenSeq = 0;
  let canceledRunToken = 0;
  let queuedInputs: string[] = [];
  let scrollRaf: number | null = null;
  let pendingSessionEvents: string[] = [];
  let sessionEventFlushTimer: number | null = null;
  let inputHistory: string[] = [];
  let historyIndex = -1;
  let historySavedInput = "";
  let persistTimer: number | null = null;
  const HISTORY_MAX = 200;

  createEffect(() => {
    const idx = slashActiveIndex();
    const container = slashListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  createEffect(() => {
    const idx = mentionActiveIndex();
    const container = mentionListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  const [showRoleForm, setShowRoleForm] = createSignal(false);
  const [roleFormName, setRoleFormName] = createSignal("Developer");
  const [roleFormRuntime, setRoleFormRuntime] = createSignal("gemini-cli");
  const [roleFormPrompt, setRoleFormPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
  const [roleFormModel, setRoleFormModel] = createSignal("");
  const [roleFormMode, setRoleFormMode] = createSignal("");
  const [roleFormAutoApprove, setRoleFormAutoApprove] = createSignal(true);
  const [editingRoleId, setEditingRoleId] = createSignal<string | null>(null);
  const [editRolePrompt, setEditRolePrompt] = createSignal("");
  const [editRoleModel, setEditRoleModel] = createSignal("");
  const [editRoleMode, setEditRoleMode] = createSignal("");
  const [editRoleAutoApprove, setEditRoleAutoApprove] = createSignal(true);
  const [editRoleMcpServersJson, setEditRoleMcpServersJson] = createSignal("[]");
  const [editRoleConfigOptionsJson, setEditRoleConfigOptionsJson] = createSignal("{}");
  const [roleFormConfigSelections, setRoleFormConfigSelections] = createSignal<Record<string, string>>({});
  const [createFormConfigOptions, setCreateFormConfigOptions] = createSignal<AcpConfigOption[]>([]);

  const [skills, setSkills] = createSignal<AppSkill[]>([]);
  const [showSkillForm, setShowSkillForm] = createSignal(false);
  const [skillFormName, setSkillFormName] = createSignal("");
  const [skillFormDescription, setSkillFormDescription] = createSignal("");
  const [skillFormContent, setSkillFormContent] = createSignal("");
  const [editingSkillId, setEditingSkillId] = createSignal<string | null>(null);

  const scheduleScrollToBottom = () => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const id = activeSessionId();
      const el = id ? document.getElementById(`msg-list-${id}`) : null;
      if (el) el.scrollTop = el.scrollHeight;
    });
  };

  const persistSessionDebounced = (sessionId: string) => {
    if (persistTimer !== null) window.clearTimeout(persistTimer);
    persistTimer = window.setTimeout(() => {
      persistTimer = null;
      const s = sessions().find((x) => x.id === sessionId);
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
    setSessions((prev) => prev.map((s) => {
      if (s.id !== id) return s;
      const trimmed = s.messages.length >= MAX_MESSAGES
        ? s.messages.slice(s.messages.length - MAX_MESSAGES + 1)
        : s.messages.slice();
      trimmed.push(message);
      return { ...s, messages: trimmed };
    }));
    scheduleScrollToBottom();
    persistSessionDebounced(id);
  };

  const pushMessage = (role: AppMessage["role"], text: string) => {
    appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role, text, at: now() });
  };

  const startStream = (roleLabel?: string) => {
    const id = `stream-${now()}`;
    const row: AppMessage = { id, role: "assistant", text: "", at: now(), roleLabel: roleLabel ?? activeSession()?.activeRole };
    patchActiveSession({ streamingMessage: row, toolCalls: new Map(), currentPlan: null, pendingPermission: null });
    scheduleScrollToBottom();
    return row;
  };

  const appendStream = (chunk: string) => {
    if (!chunk) return;
    const sid = activeSessionId();
    if (!sid) return;
    setSessions((prev) => prev.map((s) => {
      if (s.id !== sid) return s;
      if (!s.streamingMessage) return s;
      return { ...s, streamingMessage: { ...s.streamingMessage, text: s.streamingMessage.text + chunk } };
    }));
    scheduleScrollToBottom();
  };

  const flushStreamBuffer = () => {
    if (streamBuffer) {
      appendStream(streamBuffer);
      streamBuffer = "";
    }
  };

  const scheduleStreamFlush = () => {
    if (streamFlushRaf !== null) return;
    streamFlushRaf = window.requestAnimationFrame(() => {
      streamFlushRaf = null;
      flushStreamBuffer();
      if (streamBuffer) scheduleStreamFlush();
    });
  };

  const resetStreamState = () => {
    streamBuffer = "";
    if (streamFlushRaf !== null) {
      window.cancelAnimationFrame(streamFlushRaf);
      streamFlushRaf = null;
    }
  };

  const completeStream = (finalReply?: string, roleLabel?: string) => {
    flushStreamBuffer();
    const s = activeSession();
    const row = s?.streamingMessage ?? null;
    if (row) {
      const text = finalReply ?? row.text;
      appendMessage({ ...row, text, at: now() });
      patchActiveSession({ streamingMessage: null });
    } else if (finalReply) {
      appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role: "assistant", text: finalReply, at: now(), roleLabel });
    }
    resetStreamState();
    patchActiveSession({ toolCalls: new Map(), currentPlan: null, pendingPermission: null });
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
        await invoke("cancel_acp_session", { runtimeKind: assistant, roleName: role });
      } catch { }
    }
    runNextQueued();
  };

  const visibleMessages = () => {
    const rows = activeSession()?.messages ?? [];
    if (rows.length <= MESSAGE_RENDER_WINDOW) return rows;
    return rows.slice(rows.length - MESSAGE_RENDER_WINDOW);
  };

  const hiddenMessageCount = () => {
    const count = (activeSession()?.messages.length ?? 0) - MESSAGE_RENDER_WINDOW;
    return count > 0 ? count : 0;
  };

  const refreshRoles = async () => {
    try {
      const rows = await invoke<Role[]>("list_roles", { teamId: null });
      setRoles(rows);
    } catch { setRoles([]); }
  };

  const refreshSkills = async () => {
    try {
      const rows = await invoke<AppSkill[]>("list_app_skills");
      setSkills(rows);
    } catch { setSkills([]); }
  };

  const saveSkill = async () => {
    const name = skillFormName().trim();
    if (!name) return;
    try {
      await invoke("upsert_app_skill", { input: { name, description: skillFormDescription().trim(), content: skillFormContent().trim() } });
      setShowSkillForm(false);
      setEditingSkillId(null);
      setSkillFormName("");
      setSkillFormDescription("");
      setSkillFormContent("");
      await refreshSkills();
    } catch (e) { pushMessage("event", String(e)); }
  };

  const deleteSkill = async (id: string) => {
    try {
      await invoke("delete_app_skill", { id });
      await refreshSkills();
    } catch (e) { pushMessage("event", String(e)); }
  };

  const openSkillEditor = (skill: AppSkill) => {
    setEditingSkillId(skill.id);
    setSkillFormName(skill.name);
    setSkillFormDescription(skill.description);
    setSkillFormContent(skill.content);
    setShowSkillForm(true);
  };

  const fetchConfigOptions = async (runtimeKey: string): Promise<AcpConfigOption[]> => {
    try {
      const raw = await invoke<unknown[]>("list_discovered_config_options_cmd", { runtimeKey });
      return raw as AcpConfigOption[];
    } catch { return []; }
  };

  const flattenConfigValues = (opts: ConfigOptionValue[] | ConfigOptionGroup[]): ConfigOptionValue[] => {
    if (!opts || opts.length === 0) return [];
    if ("value" in opts[0]) return opts as ConfigOptionValue[];
    return (opts as ConfigOptionGroup[]).flatMap((g) => g.options);
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
      setSessions((prev) => prev.map((s) => {
        if (s.id !== sid) return s;
        const next = new Map(s.agentCommands);
        next.set(roleName, parsed);
        return { ...s, agentCommands: next };
      }));
    }).catch(() => {});
  };

  const setPreferredAssistant = (assistantKey: string | null) => {
    patchActiveSession({ selectedAssistant: assistantKey });
    if (assistantKey) window.localStorage.setItem(ASSISTANT_STORAGE_KEY, assistantKey);
    else window.localStorage.removeItem(ASSISTANT_STORAGE_KEY);
  };

  const refreshAssistants = async () => {
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
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

  const MUTATING_CMDS = ["/app_role"];
  const needsRefresh = (text: string) => MUTATING_CMDS.some((c) => text.startsWith(c));

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
          if (mentionPathCache.size > MENTION_CACHE_LIMIT) {
            const firstKey = mentionPathCache.keys().next().value;
            if (firstKey !== undefined) mentionPathCache.delete(firstKey);
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

    if (isCustomRole() && !ctx.query.startsWith("app_")) {
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
                setSessions((prev) => prev.map((sess) => {
                  if (sess.id !== sid) return sess;
                  const next = new Map(sess.agentCommands);
                  next.set(role.roleName, parsed);
                  return { ...sess, agentCommands: next };
                }));
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
      const rows = await invoke<AppMentionItem[]>("complete_cli", {
        query: ctx.query,
        limit: 20,
      });
      if (seq !== slashReqSeq) return;
      const merged = rows.slice(0, 20);
      if (merged.length === 0) {
        closeSlashMenu();
        return;
      }
      setSlashItems(merged);
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
    closeMentionMenu();
    closeSlashMenu();

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
        const sess = sessions().find((x) => x.id === sid);
        if (sess && sess.title === "New Session" && sess.messages.filter((m) => m.role === "user").length === 0) {
          const autoTitle = text.slice(0, 30);
          updateSession(sid, { title: autoTitle });
        }
      }
    }
    patchActiveSession({ submitting: true, status: "running" });

    const isAgentCmd = isCommand && !inRoleContext && /^\/(plan|act|auto|cancel)\b/.test(text);
    if (isAgentCmd) {
      const role = activeBackendRole();
      const assistant = activeSession()?.selectedAssistant ?? null;
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
        patchActiveSession({ submitting: false, status: "idle" });
        runNextQueued();
      }
      return;
    }

    let streamStarted = false;
    if ((!isUnionAiCommand) && activeSession()?.selectedAssistant) {
      streamAccepting = true;
      startStream(sendRoleLabel);
      streamStarted = true;
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: {
          input: routedText,
          selectedTeamId: null,
          selectedAssistant: activeSession()?.selectedAssistant ?? null,
          appSessionId: activeSessionId() ?? null,
        }
      });
      if (runToken <= canceledRunToken) return;
      if (res.selectedAssistant) setPreferredAssistant(res.selectedAssistant);

      if (streamStarted) {
        completeStream(res.reply, sendRoleLabel);
      } else {
        appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role: "assistant", text: res.reply, at: now(), roleLabel: sendRoleLabel });
      }

      if (needsRefresh(text)) {
        await refreshRoles();
      }
    } catch (e) {
      if (runToken <= canceledRunToken) return;
      if (streamStarted || activeSession()?.streamingMessage) {
        dropStream();
      }
      pushMessage("event", String(e));
    } finally {
      streamAccepting = false;
      if (runToken <= canceledRunToken) {
        patchActiveSession({ submitting: false, status: "idle" });
        runNextQueued();
        return;
      }
      patchActiveSession({ submitting: false, status: "done" });
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
    if (mentionDebounceTimer !== null) window.clearTimeout(mentionDebounceTimer);
    mentionDebounceTimer = window.setTimeout(() => {
      mentionDebounceTimer = null;
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

  const handleCreateRole = async () => {
    const name = roleFormName().trim();
    const runtime = roleFormRuntime() || activeSession()?.selectedAssistant || "gemini-cli";
    const prompt = roleFormPrompt().trim();
    if (!name) return;
    setShowRoleForm(false);
    await sendRaw(`/role bind ${name} ${runtime} ${prompt || "You are a helpful AI assistant."}`);
    const model = roleFormModel().trim();
    const mode = roleFormMode();
    const autoApprove = roleFormAutoApprove();
    if (model) await sendRaw(`/role edit ${name} model ${model}`, true);
    if (mode) await sendRaw(`/role edit ${name} mode ${mode}`, true);
    if (!autoApprove) await sendRaw(`/role edit ${name} auto-approve false`, true);
    const cfgSelections = roleFormConfigSelections();
    for (const [cfgId, cfgVal] of Object.entries(cfgSelections)) {
      if (cfgVal) await sendRaw(`/role edit ${name} config ${cfgId} ${cfgVal}`, true);
    }
    setRoleFormConfigSelections({});
    setCreateFormConfigOptions([]);
    await refreshRoles();
  };

  const openRoleEditor = (role: Role) => {
    setShowRoleForm(false);
    setEditingRoleId(role.id);
    setEditRolePrompt(role.systemPrompt ?? "");
    setEditRoleModel(role.model ?? "");
    setEditRoleMode(role.mode ?? "");
    setEditRoleAutoApprove(role.autoApprove);
    setEditRoleMcpServersJson(role.mcpServersJson || "[]");
    setEditRoleConfigOptionsJson(role.configOptionsJson || "{}");
    patchActiveSession({ discoveredConfigOptions: [], configOptionsLoading: true });
    invoke<unknown[]>("prewarm_role_config_cmd", {
      runtimeKind: role.runtimeKind,
      roleName: role.roleName,
      teamId: role.teamId,
    }).then((raw) => {
      patchActiveSession({ discoveredConfigOptions: raw as AcpConfigOption[], configOptionsLoading: false });
    }).catch(() => patchActiveSession({ configOptionsLoading: false }));
  };

  const closeRoleEditor = () => {
    setEditingRoleId(null);
  };

  const editingRole = () => {
    const id = editingRoleId();
    if (!id) return null;
    return roles().find((role) => role.id === id) ?? null;
  };

  const handleSaveRoleEdit = async () => {
    const role = editingRole();
    if (!role) return;
    let parsedMcp: unknown;
    let parsedConfig: unknown;
    try {
      parsedMcp = JSON.parse(editRoleMcpServersJson().trim() || "[]");
      if (!Array.isArray(parsedMcp)) throw new Error("MCP servers JSON must be an array");
    } catch (e) {
      pushMessage("event", `Invalid MCP JSON: ${String(e)}`);
      return;
    }
    try {
      parsedConfig = JSON.parse(editRoleConfigOptionsJson().trim() || "{}");
      if (!parsedConfig || typeof parsedConfig !== "object" || Array.isArray(parsedConfig)) {
        throw new Error("Config options JSON must be an object");
      }
    } catch (e) {
      pushMessage("event", `Invalid config JSON: ${String(e)}`);
      return;
    }

    const payload: RoleUpsertInput = {
      teamId: role.teamId,
      roleName: role.roleName,
      runtimeKind: role.runtimeKind,
      systemPrompt: editRolePrompt().trim(),
      model: editRoleModel().trim() || null,
      mode: editRoleMode().trim() || null,
      mcpServersJson: JSON.stringify(parsedMcp),
      configOptionsJson: JSON.stringify(parsedConfig),
      autoApprove: editRoleAutoApprove(),
    };

    try {
      await invoke<Role>("upsert_role_cmd", { input: payload });
      closeRoleEditor();
      await refreshRoles();
      pushMessage("event", `role updated: ${role.roleName}`);
    } catch (e) {
      pushMessage("event", String(e));
    }
  };

  const newSession = () => {
    const s = makeDefaultSession("New Session");
    const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
    const availableAssistant = preferred
      ? assistants().find((a) => a.key === preferred && a.available)?.key ?? null
      : assistants().find((a) => a.available)?.key ?? null;
    s.selectedAssistant = availableAssistant;
    setSessions((prev) => [...prev, s]);
    setActiveSessionId(s.id);
    void invoke("create_app_session", { title: s.title }).catch(() => {});
  };

  const closeSession = (id: string) => {
    setSessions((prev) => {
      const remaining = prev.filter((s) => s.id !== id);
      if (remaining.length === 0) return prev;
      return remaining;
    });
    if (activeSessionId() === id) {
      const remaining = sessions().filter((s) => s.id !== id);
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
        const s = makeDefaultSession("Session 1");
        loaded = [s];
        void invoke("create_app_session", { title: s.title }).catch(() => {});
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

      setSessions((prev) => prev.map((s) => {
        if (s.selectedAssistant) return s;
        return { ...s, selectedAssistant: availableAssistant };
      }));

      pushMessage("system", "Welcome to UnionAI. Agent sessions are warming up in the background.");
    })();

    void Promise.all([
      listen<AcpDeltaEvent>("acp/delta", (ev) => {
        if (!streamAccepting) return;
        const sid = activeSessionId();
        if (!sid) return;
        const s = sessions().find((x) => x.id === sid);
        if (!s?.streamingMessage) startStream();
        streamBuffer += ev.payload.delta;
        if (streamBuffer.length >= 64) {
          flushStreamBuffer();
          return;
        }
        scheduleStreamFlush();
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
      listen<AcpStreamEvent>("acp/stream", (ev) => {
        const e = ev.payload;
        switch (e.kind) {
          case "thoughtDelta":
            if (e.text && streamAccepting) {
              streamBuffer += `\n> ${e.text}`;
              scheduleStreamFlush();
            }
            break;
          case "toolCall":
            if (e.toolCallId) {
              const sid = activeSessionId();
              if (sid) setSessions((prev) => prev.map((s) => {
                if (s.id !== sid) return s;
                const next = new Map(s.toolCalls);
                next.set(e.toolCallId!, { toolCallId: e.toolCallId!, title: e.title ?? "", kind: e.toolKind ?? "unknown", status: e.status ?? "pending" });
                return { ...s, toolCalls: next };
              }));
            }
            break;
          case "toolCallUpdate":
            if (e.toolCallId) {
              const sid = activeSessionId();
              if (sid) setSessions((prev) => prev.map((s) => {
                if (s.id !== sid) return s;
                const next = new Map(s.toolCalls);
                const existing = next.get(e.toolCallId!);
                if (existing) {
                  const newContent = (e.content as unknown[]) ?? existing.content;
                  const contentJson = newContent !== existing.content
                    ? (newContent && newContent.length > 0 ? JSON.stringify(newContent, null, 2) : undefined)
                    : existing.contentJson;
                  next.set(e.toolCallId!, { ...existing, status: e.status ?? existing.status, title: e.title ?? existing.title, content: newContent, contentJson });
                }
                return { ...s, toolCalls: next };
              }));
            }
            break;
          case "plan":
            if (e.entries) patchActiveSession({ currentPlan: e.entries as AppPlanEntry[] });
            break;
          case "permissionRequest":
            if (e.requestId) {
              patchActiveSession({
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
            if (e.modeId) patchActiveSession({ currentMode: e.modeId });
            break;
          case "availableModes":
            if (e.modes) patchActiveSession({ agentModes: e.modes });
            if (e.current !== undefined) patchActiveSession({ currentMode: e.current ?? null });
            break;
          case "availableCommands":
            if (e.commands) {
              const raw = ev.payload as unknown as { role?: string };
              const roleName = raw.role;
              if (roleName) {
                const parsed = parseAgentCommands(e.commands as unknown[]);
                const sid = activeSessionId();
                if (sid) setSessions((prev) => prev.map((s) => {
                  if (s.id !== sid) return s;
                  const next = new Map(s.agentCommands);
                  next.set(roleName, parsed);
                  return { ...s, agentCommands: next };
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
      window.removeEventListener("keydown", handleGlobalKeyDown);
      dropStream();
      queuedInputs = [];
      if (mentionCloseTimer !== null) window.clearTimeout(mentionCloseTimer);
      if (mentionDebounceTimer !== null) window.clearTimeout(mentionDebounceTimer);
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

      {/* ── Tabs Bar ── */}
      <div class="flex h-9 shrink-0 items-center border-b border-white/[0.06] bg-[#0d0d10] gap-1" style="padding-left: max(8px, env(titlebar-area-x, 78px)); padding-right: 8px;">
        <For each={sessions()}>
          {(s) => (
            <div
              onClick={() => { if (renamingSessionId() !== s.id) setActiveSessionId(s.id); }}
              onDblClick={() => {
                setRenamingSessionId(s.id);
                setRenameValue(s.title);
              }}
              class={`group relative flex items-center gap-1.5 rounded-md px-3 py-1 text-xs transition-colors cursor-default select-none ${
                s.id === activeSessionId()
                  ? "bg-white/[0.08] text-white"
                  : "text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.04]"
              }`}
            >
              <Show when={s.status === "running"}>
                <span class="h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse" />
              </Show>
              <Show when={s.status === "done" && s.id !== activeSessionId()}>
                <span class="h-1.5 w-1.5 rounded-full bg-blue-500" />
              </Show>
              <Show when={renamingSessionId() === s.id} fallback={
                <span class="max-w-[120px] truncate">{s.title}</span>
              }>
                <input
                  class="max-w-[120px] bg-transparent outline-none border-b border-white/30 text-xs text-white"
                  value={renameValue()}
                  onInput={(e) => setRenameValue(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const val = renameValue().trim();
                      if (val) {
                        updateSession(s.id, { title: val });
                        void invoke("update_app_session", { id: s.id, update: { title: val } }).catch(() => {});
                      }
                      setRenamingSessionId(null);
                    } else if (e.key === "Escape") {
                      setRenamingSessionId(null);
                    }
                  }}
                  onBlur={() => {
                    const val = renameValue().trim();
                    if (val) {
                      updateSession(s.id, { title: val });
                      void invoke("update_app_session", { id: s.id, update: { title: val } }).catch(() => {});
                    }
                    setRenamingSessionId(null);
                  }}
                  ref={(el) => queueMicrotask(() => el?.select())}
                  onClick={(e) => e.stopPropagation()}
                />
              </Show>
              <Show when={sessions().length > 1}>
                <button
                  onClick={(e) => { e.stopPropagation(); closeSession(s.id); }}
                  class="ml-0.5 opacity-0 group-hover:opacity-60 hover:!opacity-100 text-zinc-400 leading-none"
                >×</button>
              </Show>
            </div>
          )}
        </For>
        <button
          onClick={() => newSession()}
          class="flex h-6 w-6 items-center justify-center rounded text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] text-sm"
        >+</button>
        <div class="flex-1" />
        <For each={assistants()}>
          {(a) => (
            <button
              onClick={() => a.available && patchActiveSession({ selectedAssistant: a.key })}
              class={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] transition-colors ${
                activeSession()?.selectedAssistant === a.key
                  ? "bg-white/[0.12] text-white"
                  : "text-zinc-600 hover:text-zinc-400"
              } ${!a.available ? "opacity-30 pointer-events-none" : ""}`}
            >
              <span class={`h-1.5 w-1.5 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
              {a.label}
            </button>
          )}
        </For>
        <Show when={(activeSession()?.agentModes ?? []).length > 0}>
          <div class="flex gap-1 ml-1">
            <For each={activeSession()?.agentModes ?? []}>
              {(m) => (
                <button
                  class={`min-h-6 rounded border px-2 py-0.5 text-[10px] ${INTERACTIVE_MOTION} ${activeSession()?.currentMode === m.id ? "border-white/25 bg-white/10 text-white" : "border-white/[0.06] text-zinc-600 hover:text-zinc-300"}`}
                  onClick={() => {
                    const assistant = activeSession()?.selectedAssistant ?? null;
                    const role = activeBackendRole();
                    if (assistant) void invoke("set_acp_mode", { runtimeKind: assistant, roleName: role, modeId: m.id });
                  }}
                >
                  {m.title ?? m.id}
                </button>
              )}
            </For>
          </div>
        </Show>
        <button
          onClick={() => { void refreshAssistants(); void refreshRoles(); void refreshSkills(); }}
          class={`flex h-7 w-7 items-center justify-center rounded-lg text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] ${INTERACTIVE_MOTION}`}
          title="Refresh"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
        </button>
        <button
          onClick={() => setShowDrawer((v) => !v)}
          class={`flex h-7 w-7 items-center justify-center rounded-lg text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] ${INTERACTIVE_MOTION}`}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <circle cx="12" cy="12" r="3"/><path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83"/>
          </svg>
        </button>
      </div>

      {/* ── Terminal Canvas ── */}
      <div
        id={`msg-list-${activeSessionId()}`}
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
        class="flex-1 overflow-auto px-6 py-4 font-mono text-sm bg-[#09090b]"
      >
        <Show when={hiddenMessageCount() > 0}>
          <div class="py-1 text-center text-xs text-zinc-600 opacity-40">
            {hiddenMessageCount()} older messages hidden for performance
          </div>
        </Show>
        <For each={visibleMessages()}>
          {(msg) => {
            if (msg.role === "user") return (
              <div class="mt-4 mb-1">
                <span class="text-zinc-500 select-none mr-2">&gt;</span>
                <span class="text-zinc-200">{msg.text}</span>
                <span class="ml-3 text-[10px] text-zinc-600">{fmt(msg.at)}</span>
              </div>
            );
            if (msg.role === "system" || msg.role === "event") return (
              <div class="flex items-center gap-3 my-1 opacity-40">
                <div class="h-px flex-1 bg-white/[0.06]" />
                <span class="text-[10px] text-zinc-500 shrink-0">{msg.text}</span>
                <div class="h-px flex-1 bg-white/[0.06]" />
              </div>
            );
            return (
              <div class="mb-3">
                <div class="flex items-center gap-2 mb-0.5">
                  <span class={`text-[10px] font-semibold ${RUNTIME_COLOR[activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-400"}`}>
                    [{msg.roleLabel ?? "assistant"}]
                  </span>
                  <span class="text-[10px] text-zinc-600">{fmt(msg.at)}</span>
                  <Show when={activeSession()?.currentMode}>
                    <span class="rounded bg-indigo-500/20 px-1 text-[9px] text-indigo-200">{activeSession()?.currentMode}</span>
                  </Show>
                </div>
                <pre class="whitespace-pre-wrap break-words text-zinc-300 leading-relaxed pl-0">{msg.text}</pre>
              </div>
            );
          }}
        </For>
        <Show when={activeSession()?.streamingMessage}>
          {(streaming) => (
            <div class="mb-3">
              <div class="flex items-center gap-2 mb-0.5">
                <span class={`text-[10px] font-semibold ${RUNTIME_COLOR[activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-400"}`}>
                  [{activeSession()?.activeRole ?? "assistant"}]
                </span>
                <span class="h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse" />
              </div>
              <pre class="whitespace-pre-wrap break-words text-zinc-300 leading-relaxed">{streaming().text}</pre>
            </div>
          )}
        </Show>
        <Show when={activeSession()?.pendingPermission}>
          {(perm) => (
            <div class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 my-2">
              <div class="mb-1 text-xs font-semibold text-amber-300">{perm().title}</div>
              <Show when={perm().description}><p class="mb-2 text-xs text-zinc-400">{perm().description}</p></Show>
              <div class="flex gap-2">
                <For each={perm().options}>{(opt) => (
                  <button
                    class={`min-h-8 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
                    onClick={() => {
                      void invoke("respond_permission", { requestId: perm().requestId, optionId: opt.optionId, cancelled: false });
                      patchActiveSession({ pendingPermission: null });
                    }}
                  >
                    {opt.title ?? opt.optionId}
                  </button>
                )}</For>
                <button
                  class={`min-h-8 rounded border border-rose-500/40 bg-rose-500/10 px-3 py-1 text-xs text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
                  onClick={() => {
                    void invoke("respond_permission", { requestId: perm().requestId, optionId: "", cancelled: true });
                    patchActiveSession({ pendingPermission: null });
                  }}
                >
                  Deny
                </button>
              </div>
            </div>
          )}
        </Show>
        <Show when={activeSession()?.currentPlan}>
          {(plan) => (
            <div class="rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 my-2">
              <div class="mb-1 text-xs font-semibold text-zinc-400">Plan</div>
              <ol class="list-inside list-decimal space-y-0.5 text-xs">
                <For each={plan()}>{(entry) => (
                  <li class="flex items-center gap-1.5">
                    <span class={`inline-block h-1.5 w-1.5 rounded-full ${entry.status === "completed" ? "bg-emerald-400" : entry.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                    <span class="text-zinc-300">{entry.title ?? entry.description ?? "step"}</span>
                  </li>
                )}</For>
              </ol>
            </div>
          )}
        </Show>
        <Show when={(activeSession()?.toolCalls.size ?? 0) > 0}>
          <div class="space-y-1 my-2">
            <For each={[...(activeSession()?.toolCalls.values() ?? [])]}>{(tc) => (
              <details class="rounded-lg border border-zinc-700 bg-zinc-900">
                <summary class="flex cursor-pointer items-center gap-2 px-3 py-1.5 text-xs">
                  <span class={`h-1.5 w-1.5 rounded-full ${tc.status === "success" || tc.status === "completed" ? "bg-emerald-400" : tc.status === "failure" || tc.status === "error" ? "bg-rose-400" : tc.status === "running" || tc.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                  <span class="font-medium text-zinc-300">{tc.title || tc.toolCallId}</span>
                  <span class="ml-auto text-zinc-500">{tc.status}</span>
                </summary>
                <Show when={tc.contentJson}>
                  <pre class="whitespace-pre-wrap break-words border-t border-zinc-700 px-3 py-1.5 text-xs text-zinc-500">
                    {tc.contentJson}
                  </pre>
                </Show>
              </details>
            )}</For>
          </div>
        </Show>
        <Show when={activeSession()?.submitting}>
          <div class="flex items-center gap-2 px-1 text-xs text-zinc-500 opacity-80 mt-2">
            <span class="h-2 w-2 rounded-full bg-white/60 animate-pulse" />
            <span>Agent is thinking...</span>
          </div>
        </Show>
      </div>

      {/* ── Capsule Input ── */}
      <div class="shrink-0 px-4 py-3 bg-[#09090b] border-t border-white/[0.04]">
        <form onSubmit={handleSend} class="relative">
          <div class="flex items-center rounded-full border border-white/[0.1] bg-white/[0.04] px-4 gap-2 focus-within:border-white/[0.18] motion-safe:transition-colors">
            <button
              type="button"
              onClick={() => patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS })}
              class={`shrink-0 py-2.5 text-xs font-mono ${isCustomRole() ? "text-blue-300 hover:text-blue-200" : "text-zinc-500"}`}
              title={isCustomRole() ? "Click to return to UnionAI" : "UnionAI mode"}
            >
              {activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS} &gt;
            </button>
            <input
              ref={(el) => { inputEl = el; }}
              value={input()}
              onInput={(e) => handleInputEvent(e.currentTarget)}
              onClick={(e) => refreshInputCompletions(e.currentTarget.value, e.currentTarget.selectionStart ?? e.currentTarget.value.length)}
              onKeyDown={handleInputKeyDown}
              onBlur={() => {
                if (mentionCloseTimer !== null) window.clearTimeout(mentionCloseTimer);
                mentionCloseTimer = window.setTimeout(() => {
                  closeMentionMenu();
                  closeSlashMenu();
                }, 120);
              }}
              onFocus={(e) => {
                if (mentionCloseTimer !== null) window.clearTimeout(mentionCloseTimer);
                if (mentionDebounceTimer !== null) {
                  window.clearTimeout(mentionDebounceTimer);
                  mentionDebounceTimer = null;
                }
                refreshInputCompletions(input(), e.currentTarget.selectionStart ?? input().length);
              }}
              placeholder={isCustomRole() ? `Chat with ${activeSession()?.activeRole}... (type / for agent commands)` : "Natural language / commands / @role @file:path"}
              class="flex-1 bg-transparent py-2.5 text-sm outline-none min-w-0 text-zinc-200 placeholder:text-zinc-600"
            />
            <button
              type="submit"
              class={`shrink-0 flex h-7 w-7 items-center justify-center rounded-full motion-safe:transition-all motion-safe:duration-150 ${input().trim() ? "bg-white text-zinc-950" : "bg-white/[0.08] text-white/20"} ${INTERACTIVE_MOTION}`}
              title={activeSession()?.submitting ? "Queue" : "Send"}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>
            </button>
          </div>
          <Show when={slashOpen() && slashItems().length > 0}>
            <div ref={slashListEl} class="absolute bottom-14 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-xl">
              <For each={slashItems()}>
                {(item, i) => (
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      applySlashCandidate(item);
                    }}
                    class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === slashActiveIndex() ? "bg-white/[0.08] text-white" : "text-zinc-400 hover:bg-white/[0.04]"}`}
                  >
                    <span class="rounded bg-indigo-500/20 px-1 text-[10px] uppercase tracking-wide text-indigo-200">cmd</span>
                    <span class="truncate font-mono text-xs">{item.value}</span>
                    <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
          <Show when={mentionOpen() && mentionItems().length > 0}>
            <div ref={mentionListEl} class="absolute bottom-14 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-xl">
              <For each={mentionItems()}>
                {(item, i) => (
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      applyMentionCandidate(item);
                    }}
                    class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === mentionActiveIndex() ? "bg-white/[0.08] text-white" : "text-zinc-400 hover:bg-white/[0.04]"}`}
                  >
                    <span class={`rounded px-1 text-[10px] uppercase tracking-wide ${item.kind === "role" ? "bg-blue-500/20 text-blue-200" : item.kind === "dir" ? "bg-emerald-500/20 text-emerald-200" : item.kind === "file" ? "bg-amber-500/20 text-amber-200" : item.kind === "command" ? "bg-indigo-500/20 text-indigo-200" : item.kind === "skill" ? "bg-violet-500/20 text-violet-200" : "bg-zinc-500/20 text-zinc-300"}`}>
                      {item.kind}
                    </span>
                    <span class="truncate font-mono text-xs">{item.value}</span>
                    <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </form>
      </div>

      {/* ── Config Drawer ── */}
      <Show when={showDrawer()}>
        <div class="absolute inset-0 z-50 flex">
          <div class="flex-1" onClick={() => setShowDrawer(false)} />
          <div class="w-72 bg-[#111114] border-l border-white/[0.07] flex flex-col overflow-hidden">
            <div class="flex items-center justify-between px-4 py-3 border-b border-white/[0.06]">
              <span class="text-xs font-semibold text-zinc-300">Config</span>
              <button onClick={() => setShowDrawer(false)} class="text-zinc-500 hover:text-zinc-300 text-lg leading-none">×</button>
            </div>
            <div class="flex-1 overflow-auto p-3 space-y-4">

              {/* Assistants */}
              <div>
                <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Assistant</div>
                <div class="space-y-1">
                  <For each={assistants()}>
                    {(a) => (
                      <button
                        class={`flex min-h-9 w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm ${INTERACTIVE_MOTION} ${
                          activeSession()?.selectedAssistant === a.key
                            ? "bg-white/[0.08] text-white"
                            : "hover:bg-white/[0.05] text-zinc-400"
                        } ${!a.available ? "opacity-40" : ""}`}
                        onClick={() => a.available && patchActiveSession({ selectedAssistant: a.key })}
                      >
                        <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
                        <span class="flex-1 font-medium">{a.label}</span>
                        <Show when={a.version}><span class="text-xs text-zinc-500">v{a.version}</span></Show>
                      </button>
                    )}
                  </For>
                </div>
              </div>

              {/* Roles */}
              <div>
                <div class="mb-2 flex items-center justify-between">
                  <span class="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Roles</span>
                  <button
                    onClick={() => setShowRoleForm((v) => {
                      const next = !v;
                      if (next) closeRoleEditor();
                      if (next && activeSession()?.selectedAssistant) setRoleFormRuntime(activeSession()!.selectedAssistant!);
                      return next;
                    })}
                    class={`min-h-7 rounded-md border border-white/[0.08] px-2 py-0.5 text-xs text-zinc-500 hover:border-white/[0.15] hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
                  >
                    {showRoleForm() ? "cancel" : "+ role"}
                  </button>
                </div>

                <Show when={showRoleForm()}>
                  <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
                    <input
                      value={roleFormName()}
                      onInput={(e) => setRoleFormName(e.currentTarget.value)}
                      placeholder="Role name (e.g. Developer)"
                      class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                    />
                    <select
                      value={roleFormRuntime()}
                      onChange={(e) => {
                        setRoleFormRuntime(e.currentTarget.value);
                        setRoleFormConfigSelections({});
                        void fetchConfigOptions(e.currentTarget.value).then(setCreateFormConfigOptions);
                      }}
                      class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                    >
                      <For each={RUNTIMES}>{(r) => <option value={r}>{r}</option>}</For>
                    </select>
                    <textarea
                      value={roleFormPrompt()}
                      onInput={(e) => setRoleFormPrompt(e.currentTarget.value)}
                      rows={3}
                      placeholder="System prompt for this role…"
                      class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
                    />
                    <For each={createFormConfigOptions()}>
                      {(opt) => {
                        const values = () => flattenConfigValues(opt.options);
                        const selected = () => roleFormConfigSelections()[opt.id] ?? "";
                        const isModel = () => opt.category === "model" || opt.id === "model";
                        const isMode = () => opt.category === "mode" || opt.id === "mode";
                        return (
                          <Show when={!isModel() && !isMode()}>
                            <div class="flex flex-col gap-0.5">
                              <label class="text-[10px] text-zinc-500">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                              <select
                                value={selected()}
                                onChange={(e) => setRoleFormConfigSelections((prev) => ({ ...prev, [opt.id]: e.currentTarget.value }))}
                                class="h-8 w-full rounded border border-zinc-700 bg-zinc-900 px-2 text-xs text-zinc-200"
                              >
                                <option value="">(default: {opt.currentValue})</option>
                                <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                              </select>
                            </div>
                          </Show>
                        );
                      }}
                    </For>
                    <Show when={createFormConfigOptions().some((o) => o.category === "model" || o.id === "model")}>
                      {(() => {
                        const modelOpt = () => createFormConfigOptions().find((o) => o.category === "model" || o.id === "model")!;
                        const values = () => flattenConfigValues(modelOpt().options);
                        return (
                          <select
                            value={roleFormModel()}
                            onChange={(e) => setRoleFormModel(e.currentTarget.value)}
                            class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                          >
                            <option value="">Model (default: {modelOpt().currentValue})</option>
                            <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                          </select>
                        );
                      })()}
                    </Show>
                    <Show when={!createFormConfigOptions().some((o) => o.category === "model" || o.id === "model")}>
                      <input
                        value={roleFormModel()}
                        onInput={(e) => setRoleFormModel(e.currentTarget.value)}
                        placeholder="Model (optional)"
                        class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                      />
                    </Show>
                    <Show when={createFormConfigOptions().some((o) => o.category === "mode" || o.id === "mode")}>
                      {(() => {
                        const modeOpt = () => createFormConfigOptions().find((o) => o.category === "mode" || o.id === "mode")!;
                        const values = () => flattenConfigValues(modeOpt().options);
                        return (
                          <select
                            value={roleFormMode()}
                            onChange={(e) => setRoleFormMode(e.currentTarget.value)}
                            class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                          >
                            <option value="">Mode (default: {modeOpt().currentValue})</option>
                            <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                          </select>
                        );
                      })()}
                    </Show>
                    <Show when={!createFormConfigOptions().some((o) => o.category === "mode" || o.id === "mode")}>
                      <select
                        value={roleFormMode()}
                        onChange={(e) => setRoleFormMode(e.currentTarget.value)}
                        class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                      >
                        <option value="">Mode (default)</option>
                        <option value="plan">plan</option>
                        <option value="act">act</option>
                        <option value="auto">auto</option>
                      </select>
                    </Show>
                    <label class="flex items-center gap-2 text-xs text-zinc-500">
                      <input
                        type="checkbox"
                        checked={roleFormAutoApprove()}
                        onChange={(e) => setRoleFormAutoApprove(e.currentTarget.checked)}
                        class="rounded"
                      />
                      Auto-approve permissions
                    </label>
                    <button
                      onClick={() => void handleCreateRole()}
                      class={`h-9 w-full rounded-lg bg-white text-sm font-semibold text-zinc-950 ${INTERACTIVE_MOTION}`}
                    >
                      Create Role
                    </button>
                  </div>
                </Show>

                <Show when={editingRole()}>
                  {(role) => (
                    <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
                      <div class="flex items-center justify-between">
                        <div class="text-xs font-semibold text-zinc-200">Edit {role().roleName}</div>
                        <button
                          onClick={closeRoleEditor}
                          class={`rounded px-1.5 py-0.5 text-xs text-zinc-500 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
                        >
                          close
                        </button>
                      </div>
                      <div class="text-[10px] text-zinc-600">provider locked: {role().runtimeKind}</div>
                      <textarea
                        value={editRolePrompt()}
                        onInput={(e) => setEditRolePrompt(e.currentTarget.value)}
                        rows={3}
                        placeholder="System prompt"
                        class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
                      />
                      {(() => {
                        const opts = activeSession()?.discoveredConfigOptions ?? [];
                        const currentCfg = (): Record<string, string> => {
                          try { return JSON.parse(editRoleConfigOptionsJson() || "{}"); } catch { return {}; }
                        };
                        const updateCfg = (id: string, val: string) => {
                          const map = { ...currentCfg() };
                          if (val) map[id] = val; else delete map[id];
                          setEditRoleConfigOptionsJson(JSON.stringify(map));
                        };
                        const modelOpt = () => opts.find((o) => o.category === "model" || o.id === "model");
                        const modeOpt = () => opts.find((o) => o.category === "mode" || o.id === "mode");
                        const otherOpts = () => opts.filter((o) => o.id !== "model" && o.id !== "mode" && o.category !== "model" && o.category !== "mode");
                        return (
                          <>
                            <Show when={modelOpt()} fallback={
                              <input value={editRoleModel()} onInput={(e) => setEditRoleModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200" />
                            }>
                              {(mo) => {
                                const values = () => flattenConfigValues(mo().options);
                                return (
                                  <select value={editRoleModel() || mo().currentValue} onChange={(e) => setEditRoleModel(e.currentTarget.value)} class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200">
                                    <option value="">Model (default: {mo().currentValue})</option>
                                    <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                                  </select>
                                );
                              }}
                            </Show>
                            <Show when={modeOpt()} fallback={
                              <input value={editRoleMode()} onInput={(e) => setEditRoleMode(e.currentTarget.value)} placeholder="Mode (optional)" class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200" />
                            }>
                              {(mo) => {
                                const values = () => flattenConfigValues(mo().options);
                                return (
                                  <select value={editRoleMode() || mo().currentValue} onChange={(e) => setEditRoleMode(e.currentTarget.value)} class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200">
                                    <option value="">Mode (default: {mo().currentValue})</option>
                                    <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                                  </select>
                                );
                              }}
                            </Show>
                            <For each={otherOpts()}>
                              {(opt) => {
                                const values = () => flattenConfigValues(opt.options);
                                const selected = () => currentCfg()[opt.id] ?? "";
                                return (
                                  <div class="flex flex-col gap-0.5">
                                    <label class="text-[10px] text-zinc-500">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                                    <select value={selected()} onChange={(e) => updateCfg(opt.id, e.currentTarget.value)} class="h-8 w-full rounded border border-zinc-700 bg-zinc-900 px-2 text-xs text-zinc-200">
                                      <option value="">(default: {opt.currentValue})</option>
                                      <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                                    </select>
                                  </div>
                                );
                              }}
                            </For>
                            <Show when={opts.length === 0}>
                              <div class="text-[10px] text-zinc-600 italic">
                                {activeSession()?.configOptionsLoading ? "Loading config options from agent..." : "No config options available for this agent."}
                              </div>
                            </Show>
                          </>
                        );
                      })()}
                      <textarea
                        value={editRoleMcpServersJson()}
                        onInput={(e) => setEditRoleMcpServersJson(e.currentTarget.value)}
                        rows={2}
                        placeholder='MCP servers JSON (e.g. [{"name":"..." }])'
                        class="w-full resize-y rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-xs font-mono text-zinc-300"
                      />
                      <label class="flex items-center gap-2 text-xs text-zinc-500">
                        <input
                          type="checkbox"
                          checked={editRoleAutoApprove()}
                          onChange={(e) => setEditRoleAutoApprove(e.currentTarget.checked)}
                          class="rounded"
                        />
                        Auto-approve permissions
                      </label>
                      <button
                        onClick={() => void handleSaveRoleEdit()}
                        class={`h-9 w-full rounded-lg bg-white text-sm font-semibold text-zinc-950 ${INTERACTIVE_MOTION}`}
                      >
                        Save Role
                      </button>
                    </div>
                  )}
                </Show>

                <div class="space-y-1">
                  <For each={roles()}>
                    {(role) => (
                      <div class="group relative flex items-start gap-1.5 rounded-lg border-l-2 border-l-transparent px-2 py-1.5 motion-safe:transition-all motion-safe:duration-150 hover:border-l-white/20 hover:bg-white/[0.04]">
                        <div class="flex-1 min-w-0">
                          <div class="flex items-center gap-1.5 flex-wrap">
                            <span class="text-xs font-medium text-zinc-300">{role.roleName}</span>
                            <span class={`text-xs ${RUNTIME_COLOR[role.runtimeKind] ?? "text-zinc-500"}`}>{role.runtimeKind}</span>
                            <Show when={role.model}><span class="rounded bg-blue-500/20 px-1 text-[9px] text-blue-200">{role.model}</span></Show>
                            <Show when={role.mode}><span class="rounded bg-indigo-500/20 px-1 text-[9px] text-indigo-200">{role.mode}</span></Show>
                            <Show when={!role.autoApprove}><span class="rounded bg-amber-500/20 px-1 text-[9px] text-amber-200">manual</span></Show>
                            {(() => {
                              try {
                                const cfg = JSON.parse(role.configOptionsJson || "{}");
                                const keys = Object.keys(cfg).filter((k) => k !== "model" && k !== "mode" && cfg[k]);
                                return keys.length > 0 ? <span class="rounded bg-teal-500/20 px-1 text-[9px] text-teal-200">{keys.length} cfg</span> : null;
                              } catch { return null; }
                            })()}
                          </div>
                          <p class="truncate text-xs text-zinc-600">{role.systemPrompt}</p>
                        </div>
                        <div class="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100">
                          <button
                            onClick={() => openRoleEditor(role)}
                            class={`rounded px-1.5 py-0.5 text-xs ${editingRoleId() === role.id ? "text-blue-300" : "text-zinc-500 hover:text-zinc-200"} ${INTERACTIVE_MOTION}`}
                          >
                            {editingRoleId() === role.id ? "editing" : "edit"}
                          </button>
                        </div>
                      </div>
                    )}
                  </For>
                  <Show when={roles().length === 0 && !showRoleForm()}>
                    <p class="rounded-lg border border-dashed border-zinc-700 p-2 text-center text-xs text-zinc-600">
                      No roles yet. Click "+ role" to add one.
                    </p>
                  </Show>
                </div>
              </div>

              {/* Skills */}
              <div>
                <div class="mb-2 flex items-center justify-between">
                  <span class="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Skills</span>
                  <button
                    onClick={() => {
                      setEditingSkillId(null);
                      setSkillFormName("");
                      setSkillFormDescription("");
                      setSkillFormContent("");
                      setShowSkillForm((v) => !v);
                    }}
                    class={`min-h-7 rounded-md border border-white/[0.08] px-2 py-0.5 text-xs text-zinc-500 hover:border-white/[0.15] hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
                  >
                    {showSkillForm() && !editingSkillId() ? "cancel" : "+ skill"}
                  </button>
                </div>

                <Show when={showSkillForm()}>
                  <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
                    <input
                      value={skillFormName()}
                      onInput={(e) => setSkillFormName(e.currentTarget.value)}
                      placeholder="Skill name (e.g. code-review)"
                      class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                    />
                    <input
                      value={skillFormDescription()}
                      onInput={(e) => setSkillFormDescription(e.currentTarget.value)}
                      placeholder="Short description"
                      class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
                    />
                    <textarea
                      value={skillFormContent()}
                      onInput={(e) => setSkillFormContent(e.currentTarget.value)}
                      rows={4}
                      placeholder="Skill content — prompt instructions, templates, context…"
                      class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
                    />
                    <button
                      onClick={saveSkill}
                      class={`w-full rounded-lg bg-indigo-600 py-1.5 text-xs font-medium text-white hover:bg-indigo-500 ${INTERACTIVE_MOTION}`}
                    >
                      {editingSkillId() ? "Save changes" : "Create skill"}
                    </button>
                  </div>
                </Show>

                <div class="space-y-1">
                  <For each={skills()}>
                    {(skill) => (
                      <div class="group rounded-lg border border-white/[0.05] bg-white/[0.02] px-2.5 py-2">
                        <div class="flex items-center justify-between gap-1">
                          <span class="text-xs font-medium text-zinc-300">#{skill.name}</span>
                          <div class="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                            <button
                              onClick={() => openSkillEditor(skill)}
                              class="rounded px-1.5 py-0.5 text-[10px] text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.06]"
                            >edit</button>
                            <button
                              onClick={() => void deleteSkill(skill.id)}
                              class="rounded px-1.5 py-0.5 text-[10px] text-rose-600 hover:text-rose-400 hover:bg-rose-500/10"
                            >del</button>
                          </div>
                        </div>
                        <Show when={skill.description}>
                          <p class="mt-0.5 text-[10px] text-zinc-600 truncate">{skill.description}</p>
                        </Show>
                      </div>
                    )}
                  </For>
                  <Show when={skills().length === 0 && !showSkillForm()}>
                    <p class="rounded-lg border border-dashed border-zinc-700 p-2 text-center text-xs text-zinc-600">
                      No skills yet. Click "+ skill" to add one.
                    </p>
                  </Show>
                </div>
              </div>

              {/* Quick actions */}
              <div class="flex flex-wrap gap-0.5 border-t border-white/[0.05] pt-3">
                <button onClick={() => void sendRaw("/workflow list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">workflows</button>
                <button onClick={() => void sendRaw("/context list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">context</button>
                <button onClick={() => void sendRaw("/session list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">sessions</button>
              </div>

            </div>
          </div>
        </div>
      </Show>

    </div>
  );
}
