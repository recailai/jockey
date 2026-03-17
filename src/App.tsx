import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";

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
type ToolCallState = { toolCallId: string; title: string; kind: string; status: string; content?: unknown[]; contentJson?: string };
type PlanEntry = { title?: string; status?: string; description?: string };
type PermissionState = { requestId: string; title: string; description: string | null; options: Array<{ optionId: string; title?: string }> };
type AcpStreamEvent = {
  kind: string;
  text?: string;
  toolCallId?: string; title?: string; toolKind?: string; status?: string; content?: unknown[];
  entries?: PlanEntry[];
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
type ChatMessage = { id: string; role: "system" | "user" | "assistant" | "event"; text: string; at: number };
type MentionCandidate = { value: string; kind: "role" | "file" | "dir" | "hint" | "command"; detail: string };

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

export default function App() {
  const [roles, setRoles] = createSignal<Role[]>([]);
  const [assistants, setAssistants] = createSignal<AssistantRuntime[]>([]);
  const [selectedAssistant, setSelectedAssistant] = createSignal<string | null>(null);
  const [input, setInput] = createSignal("");
  const [toolCalls, setToolCalls] = createSignal<Map<string, ToolCallState>>(new Map());
  const [currentPlan, setCurrentPlan] = createSignal<PlanEntry[] | null>(null);
  const [pendingPermission, setPendingPermission] = createSignal<PermissionState | null>(null);
  const [agentModes, setAgentModes] = createSignal<Array<{ id: string; title?: string }>>([]);
  const [currentMode, setCurrentMode] = createSignal<string | null>(null);
  const [mentionOpen, setMentionOpen] = createSignal(false);
  const [mentionItems, setMentionItems] = createSignal<MentionCandidate[]>([]);
  const [mentionActiveIndex, setMentionActiveIndex] = createSignal(0);
  const [mentionRange, setMentionRange] = createSignal<{ start: number; end: number; query: string } | null>(null);
  const [slashOpen, setSlashOpen] = createSignal(false);
  const [slashItems, setSlashItems] = createSignal<MentionCandidate[]>([]);
  const [slashActiveIndex, setSlashActiveIndex] = createSignal(0);
  const [slashRange, setSlashRange] = createSignal<{ end: number; query: string } | null>(null);
  const [activeRole, setActiveRole] = createSignal(DEFAULT_ROLE_ALIAS);
  const [submitting, setSubmitting] = createSignal(false);
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [streamingMessage, setStreamingMessage] = createSignal<ChatMessage | null>(null);
  let mentionReqSeq = 0;
  let slashReqSeq = 0;
  let mentionCloseTimer: number | null = null;
  let mentionDebounceTimer: number | null = null;
  let mentionPathCache = new Map<string, MentionCandidate[]>();
  let inputEl: HTMLInputElement | undefined;
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
  const HISTORY_MAX = 200;

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
  const [discoveredConfigOptions, setDiscoveredConfigOptions] = createSignal<AcpConfigOption[]>([]);
  const [configOptionsLoading, setConfigOptionsLoading] = createSignal(false);
  const [roleFormConfigSelections, setRoleFormConfigSelections] = createSignal<Record<string, string>>({});
  const [createFormConfigOptions, setCreateFormConfigOptions] = createSignal<AcpConfigOption[]>([]);
  const [agentCommands, setAgentCommands] = createSignal<Map<string, Array<{ name: string; description: string; hint?: string }>>>(new Map());

  const scheduleScrollToBottom = () => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const el = document.getElementById("msg-list");
      if (el) el.scrollTop = el.scrollHeight;
    });
  };

  const appendMessage = (message: ChatMessage) => {
    setMessages((prev) => {
      const trimmed =
        prev.length >= MAX_MESSAGES
          ? prev.slice(prev.length - MAX_MESSAGES + 1)
          : prev.slice();
      trimmed.push(message);
      return trimmed;
    });
    scheduleScrollToBottom();
  };

  const pushMessage = (role: ChatMessage["role"], text: string) => {
    appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, role, text, at: now() });
  };

  const startStream = () => {
    const id = `stream-${now()}`;
    const row: ChatMessage = { id, role: "assistant", text: "", at: now() };
    setStreamingMessage(row);
    setToolCalls(new Map());
    setCurrentPlan(null);
    setPendingPermission(null);
    scheduleScrollToBottom();
    return row;
  };

  const appendStream = (chunk: string) => {
    if (!chunk) return;
    setStreamingMessage((prev) => (prev ? { ...prev, text: prev.text + chunk } : prev));
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

  const completeStream = (finalReply?: string) => {
    flushStreamBuffer();
    const row = streamingMessage();
    if (row) {
      const text = finalReply ?? row.text;
      appendMessage({ ...row, text, at: now() });
      setStreamingMessage(null);
    } else if (finalReply) {
      pushMessage("assistant", finalReply);
    }
    resetStreamState();
    setToolCalls(new Map());
    setCurrentPlan(null);
    setPendingPermission(null);
  };

  const dropStream = () => {
    setStreamingMessage(null);
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
    if (submitting()) return;
    const next = queuedInputs.shift();
    if (!next) return;
    void sendRaw(next);
  };

  const isCustomRole = () => activeRole() !== DEFAULT_ROLE_ALIAS && activeRole() !== DEFAULT_BACKEND_ROLE;
  const activeBackendRole = () => isCustomRole() ? activeRole() : DEFAULT_BACKEND_ROLE;

  const cancelCurrentRun = async () => {
    if (!submitting()) return;
    canceledRunToken = Math.max(canceledRunToken, runTokenSeq);
    streamAccepting = false;
    dropStream();
    setToolCalls(new Map());
    setCurrentPlan(null);
    setPendingPermission(null);
    setSubmitting(false);
    pushMessage("event", "Cancellation requested.");
    const assistant = selectedAssistant();
    const role = activeBackendRole();
    if (assistant) {
      try {
        await invoke("cancel_acp_session", { runtimeKind: assistant, roleName: role });
      } catch { /* best effort */ }
    }
    runNextQueued();
  };

  const visibleMessages = () => {
    const rows = messages();
    if (rows.length <= MESSAGE_RENDER_WINDOW) return rows;
    return rows.slice(rows.length - MESSAGE_RENDER_WINDOW);
  };

  const hiddenMessageCount = () => {
    const count = messages().length - MESSAGE_RENDER_WINDOW;
    return count > 0 ? count : 0;
  };

  const refreshRoles = async () => {
    try {
      const rows = await invoke<Role[]>("list_roles", { teamId: null });
      setRoles(rows);
    } catch { setRoles([]); }
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

  const fetchAndCacheAgentCommands = (runtimeKey: string) => {
    void invoke<unknown[]>("list_available_commands_cmd", { runtimeKey }).then((raw) => {
      const parsed = parseAgentCommands(raw);
      setAgentCommands((prev) => { const next = new Map(prev); next.set(runtimeKey, parsed); return next; });
    }).catch(() => {});
  };

  const setPreferredAssistant = (assistantKey: string | null) => {
    setSelectedAssistant(assistantKey);
    if (assistantKey) window.localStorage.setItem(ASSISTANT_STORAGE_KEY, assistantKey);
    else window.localStorage.removeItem(ASSISTANT_STORAGE_KEY);
  };

  const refreshAssistants = async () => {
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
    const preferred = window.localStorage.getItem(ASSISTANT_STORAGE_KEY);
    const current = selectedAssistant();
    const currentAvailable = current ? rows.find((a) => a.key === current && a.available) : null;
    if (currentAvailable) return;
    const preferredAvailable = preferred ? rows.find((a) => a.key === preferred && a.available) : null;
    const first = preferredAvailable ?? rows.find((a) => a.available) ?? null;
    if (first) {
      setPreferredAssistant(first.key);
    }
  };

  const MUTATING_CMDS = ["/role", "/init", "/workflow", "/context"];
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
    const at = left.lastIndexOf("@");
    if (at < 0) return null;
    const prev = at > 0 ? left[at - 1] : " ";
    if (!/\s/.test(prev)) return null;
    const query = left.slice(at + 1);
    if (/\s/.test(query)) return null;
    let end = caret;
    while (end < text.length && !/\s/.test(text[end])) end += 1;
    return { start: at + 1, end, query };
  };

  const listRoleMentionCandidates = (query: string) => {
    const q = query.toLowerCase();
    const out: MentionCandidate[] = [];
    const rolePrefix = query.startsWith("role:");
    const roleQuery = rolePrefix ? query.slice(5).toLowerCase() : q;
    for (const role of roles()) {
      const nameLower = role.roleName.toLowerCase();
      if (roleQuery && !nameLower.includes(roleQuery)) continue;
      out.push({
        value: rolePrefix || query.length === 0 ? `role:${role.roleName}` : role.roleName,
        kind: "role",
        detail: role.runtimeKind,
      });
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
    const staticItems: MentionCandidate[] = [];
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
          const rows = await invoke<MentionCandidate[]>("complete_mentions", {
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
          // Keep role suggestions only.
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

  const buildAgentSlashCandidates = (runtimeKey: string, query: string): MentionCandidate[] => {
    const queryLower = query.toLowerCase().replace(/^\//, "");
    const out: MentionCandidate[] = [];
    const cmds = agentCommands().get(runtimeKey) ?? [];
    for (const cmd of cmds) {
      const value = `/${cmd.name}`;
      if (queryLower && !cmd.name.toLowerCase().includes(queryLower)) continue;
      out.push({ value, kind: "command", detail: cmd.description });
    }
    for (const opt of discoveredConfigOptions()) {
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

    if (isCustomRole()) {
      const role = roles().find((r) => r.roleName === activeRole());
      if (role) {
        if (discoveredConfigOptions().length === 0) {
          const opts = await fetchConfigOptions(role.runtimeKind);
          if (seq !== slashReqSeq) return;
          setDiscoveredConfigOptions(opts);
        }
        if ((agentCommands().get(role.runtimeKind) ?? []).length === 0) {
          fetchAndCacheAgentCommands(role.runtimeKind);
        }
        const candidates = buildAgentSlashCandidates(role.runtimeKind, ctx.query);
        if (seq !== slashReqSeq) return;
        if (candidates.length === 0) { closeSlashMenu(); return; }
        setSlashItems(candidates);
        setSlashActiveIndex(0);
        setSlashOpen(true);
        return;
      }
    }

    try {
      const rows = await invoke<MentionCandidate[]>("complete_cli", {
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

  const applyMentionCandidate = (item: MentionCandidate) => {
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

  const applySlashCandidate = (item: MentionCandidate) => {
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

    const isCommand = text.startsWith("/");
    const inRoleContext = isCustomRole();
    const isUnionAiCommand = isCommand && !inRoleContext;
    const isRoleSlashCmd = isCommand && inRoleContext;
    let routedText = text;

    if (!isCommand) {
      const mentionMatch = text.match(/^@(\S+)/);
      if (mentionMatch) {
        const target = mentionMatch[1];
        if (
          target === "assistant" ||
          target === DEFAULT_ROLE_ALIAS ||
          target === DEFAULT_BACKEND_ROLE
        ) {
          setActiveRole(DEFAULT_ROLE_ALIAS);
          routedText = text.replace(/^@\S+\s*/, "").trim();
        } else {
          setActiveRole(target);
          setDiscoveredConfigOptions([]);
          const targetRole = roles().find((r) => r.roleName === target);
          if (targetRole) {
            void fetchConfigOptions(targetRole.runtimeKind).then(setDiscoveredConfigOptions);
            fetchAndCacheAgentCommands(targetRole.runtimeKind);
          }
        }
      } else if (isCustomRole() && !text.startsWith("@")) {
        routedText = `@${activeRole()} ${text}`;
      }
    }

    if (isRoleSlashCmd) {
      routedText = `@${activeRole()} ${text}`;
    }

    if (!silent) pushMessage("user", text);
    setSubmitting(true);

    const isAgentCmd = isCommand && !inRoleContext && /^\/(plan|act|auto|cancel)\b/.test(text);
    if (isAgentCmd) {
      const role = activeBackendRole();
      const assistant = selectedAssistant();
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
        setSubmitting(false);
        runNextQueued();
      }
      return;
    }

    let streamStarted = false;
    if ((!isUnionAiCommand) && selectedAssistant()) {
      streamAccepting = true;
      startStream();
      streamStarted = true;
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: {
          input: routedText,
          selectedTeamId: null,
          selectedAssistant: selectedAssistant(),
        }
      });
      if (runToken <= canceledRunToken) return;
      if (res.selectedAssistant) setPreferredAssistant(res.selectedAssistant);

      if (streamStarted) {
        completeStream(res.reply);
      } else {
        pushMessage("assistant", res.reply);
      }

      if (needsRefresh(text)) {
        await refreshRoles();
      }
    } catch (e) {
      if (runToken <= canceledRunToken) return;
      if (streamStarted || streamingMessage()) {
        dropStream();
      }
      pushMessage("event", String(e));
    } finally {
      streamAccepting = false;
      if (runToken <= canceledRunToken) {
        setSubmitting(false);
        runNextQueued();
        return;
      }
      setSubmitting(false);
      runNextQueued();
    }
  };

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const text = input().trim();
    if (!text) return;
    if (submitting()) {
      queuedInputs.push(text);
      setInput("");
      pushMessage("system", `Queued (${queuedInputs.length}): ${text}`);
      return;
    }
    if (!selectedAssistant() && !isCustomRole() && !text.startsWith("/")) {
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
    if (e.key === "Escape" && submitting()) {
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
    const runtime = roleFormRuntime() || selectedAssistant() || "gemini-cli";
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
    setDiscoveredConfigOptions([]);
    setConfigOptionsLoading(true);
    invoke<unknown[]>("prewarm_role_config_cmd", {
      runtimeKind: role.runtimeKind,
      roleName: role.roleName,
      teamId: role.teamId,
    }).then((raw) => {
      setDiscoveredConfigOptions(raw as AcpConfigOption[]);
      setConfigOptionsLoading(false);
    }).catch(() => setConfigOptionsLoading(false));
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

  onMount(() => {
    const handlers: UnlistenFn[] = [];
    pushMessage("system", "Welcome to UnionAI. Agent sessions are warming up in the background.");

    void (async () => {
      await refreshAssistants();
      await refreshRoles();
    })();

    void Promise.all([
      listen<AcpDeltaEvent>("acp/delta", (ev) => {
        if (!streamAccepting) return;
        if (!streamingMessage()) startStream();
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
              setToolCalls((prev) => {
                const next = new Map(prev);
                next.set(e.toolCallId!, { toolCallId: e.toolCallId!, title: e.title ?? "", kind: e.toolKind ?? "unknown", status: e.status ?? "pending" });
                return next;
              });
            }
            break;
          case "toolCallUpdate":
            if (e.toolCallId) {
              setToolCalls((prev) => {
                const next = new Map(prev);
                const existing = next.get(e.toolCallId!);
                if (existing) {
                  const newContent = (e.content as unknown[]) ?? existing.content;
                  const contentJson = newContent !== existing.content
                    ? (newContent && newContent.length > 0 ? JSON.stringify(newContent, null, 2) : undefined)
                    : existing.contentJson;
                  next.set(e.toolCallId!, { ...existing, status: e.status ?? existing.status, title: e.title ?? existing.title, content: newContent, contentJson });
                }
                return next;
              });
            }
            break;
          case "plan":
            if (e.entries) setCurrentPlan(e.entries as PlanEntry[]);
            break;
          case "permissionRequest":
            if (e.requestId) {
              setPendingPermission({
                requestId: e.requestId,
                title: e.title ?? "Permission Required",
                description: e.description ?? null,
                options: (e.options as Array<{ optionId: string; title?: string }>) ?? [],
              });
            }
            break;
          case "modeUpdate":
            if (e.modeId) setCurrentMode(e.modeId);
            break;
          case "availableModes":
            if (e.modes) setAgentModes(e.modes);
            if (e.current !== undefined) setCurrentMode(e.current ?? null);
            break;
          case "availableCommands":
            if (e.commands) {
              const raw = ev.payload as unknown as { role?: string };
              const roleName = raw.role;
              if (roleName) {
                const role = roles().find((r) => r.roleName === roleName);
                if (role) {
                  const parsed = parseAgentCommands(e.commands as unknown[]);
                  setAgentCommands((prev) => {
                    const next = new Map(prev);
                    next.set(role.runtimeKind, parsed);
                    return next;
                  });
                }
              }
            }
            break;
          default:
            break;
        }
      }),
    ]).then((hs) => handlers.push(...hs));

    onCleanup(() => {
      dropStream();
      queuedInputs = [];
      if (mentionCloseTimer !== null) window.clearTimeout(mentionCloseTimer);
      if (mentionDebounceTimer !== null) window.clearTimeout(mentionDebounceTimer);
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
    <div class="window-bg h-dvh overflow-hidden text-[var(--ui-text)]">
      <div class="mx-auto grid h-full max-w-[1600px] grid-cols-1 gap-0 px-0 py-0 lg:grid-cols-[260px_1fr]">

        {/* ── Sidebar ── */}
        <aside class="flex h-full flex-col overflow-hidden bg-[#18181b] border-r border-white/[0.06] p-0">

          {/* Assistants */}
          <div class="shrink-0 border-b border-[var(--ui-border)] p-3">
            <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-[var(--ui-muted)]">Assistant</div>
            <div class="space-y-1">
              <For each={assistants()}>
                {(a) => (
                  <button
                    class={`flex min-h-9 w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm ${INTERACTIVE_MOTION} ${
                      selectedAssistant() === a.key
                        ? "sidebar-item-selected bg-white/[0.08] pl-4"
                        : "hover:bg-white/[0.05]"
                    } ${!a.available ? "opacity-40" : ""}`}
                    onClick={() => a.available && setPreferredAssistant(a.key)}
                  >
                    <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
                    <span class="flex-1 font-medium">{a.label}</span>
                    <Show when={a.version}><span class="text-xs text-[var(--ui-muted)]">v{a.version}</span></Show>
                  </button>
                )}
              </For>
            </div>
          </div>

          {/* Roles */}
          <div class="flex flex-1 flex-col overflow-hidden p-3">
            <div class="mb-2 flex items-center justify-between">
              <span class="text-[11px] font-semibold uppercase tracking-wider text-[var(--ui-muted)]">Roles</span>
              <button
                onClick={() => setShowRoleForm((v) => {
                  const next = !v;
                  if (next) closeRoleEditor();
                  if (next && selectedAssistant()) setRoleFormRuntime(selectedAssistant()!);
                  return next;
                })}
                class={`min-h-8 rounded-md border border-white/[0.08] px-2 py-1 text-xs text-[var(--ui-muted)] hover:border-white/[0.15] hover:text-[var(--ui-text)] ${INTERACTIVE_MOTION}`}
              >
                {showRoleForm() ? "cancel" : "+ role"}
              </button>
            </div>

            {/* Role creation form */}
            <Show when={showRoleForm()}>
              <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
                <input
                  value={roleFormName()}
                  onInput={(e) => setRoleFormName(e.currentTarget.value)}
                  placeholder="Role name (e.g. Developer)"
                  class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
                />
                <select
                  value={roleFormRuntime()}
                  onChange={(e) => {
                    setRoleFormRuntime(e.currentTarget.value);
                    setRoleFormConfigSelections({});
                    void fetchConfigOptions(e.currentTarget.value).then(setCreateFormConfigOptions);
                  }}
                  class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
                >
                  <For each={RUNTIMES}>{(r) => <option value={r}>{r}</option>}</For>
                </select>
                <textarea
                  value={roleFormPrompt()}
                  onInput={(e) => setRoleFormPrompt(e.currentTarget.value)}
                  rows={3}
                  placeholder="System prompt for this role…"
                  class="w-full resize-none rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 py-2 text-sm"
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
                          <label class="text-[10px] text-[var(--ui-muted)]">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                          <select
                            value={selected()}
                            onChange={(e) => setRoleFormConfigSelections((prev) => ({ ...prev, [opt.id]: e.currentTarget.value }))}
                            class="h-8 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 text-xs"
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
                        class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
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
                    class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
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
                        class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
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
                    class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm"
                  >
                    <option value="">Mode (default)</option>
                    <option value="plan">plan</option>
                    <option value="act">act</option>
                    <option value="auto">auto</option>
                  </select>
                </Show>
                <label class="flex items-center gap-2 text-xs text-[var(--ui-muted)]">
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
                    <div class="text-xs font-semibold text-[var(--ui-text)]">Edit {role().roleName}</div>
                    <button
                      onClick={closeRoleEditor}
                      class={`rounded px-1.5 py-0.5 text-xs text-[var(--ui-muted)] hover:text-[var(--ui-text)] ${INTERACTIVE_MOTION}`}
                    >
                      close
                    </button>
                  </div>
                  <div class="text-[10px] text-[var(--ui-muted)]">provider locked: {role().runtimeKind}</div>
                  <textarea
                    value={editRolePrompt()}
                    onInput={(e) => setEditRolePrompt(e.currentTarget.value)}
                    rows={3}
                    placeholder="System prompt"
                    class="w-full resize-none rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 py-2 text-sm"
                  />
                  {(() => {
                    const opts = discoveredConfigOptions();
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
                          <input value={editRoleModel()} onInput={(e) => setEditRoleModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm" />
                        }>
                          {(mo) => {
                            const values = () => flattenConfigValues(mo().options);
                            return (
                              <select value={editRoleModel() || mo().currentValue} onChange={(e) => setEditRoleModel(e.currentTarget.value)} class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm">
                                <option value="">Model (default: {mo().currentValue})</option>
                                <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                              </select>
                            );
                          }}
                        </Show>
                        <Show when={modeOpt()} fallback={
                          <input value={editRoleMode()} onInput={(e) => setEditRoleMode(e.currentTarget.value)} placeholder="Mode (optional)" class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm" />
                        }>
                          {(mo) => {
                            const values = () => flattenConfigValues(mo().options);
                            return (
                              <select value={editRoleMode() || mo().currentValue} onChange={(e) => setEditRoleMode(e.currentTarget.value)} class="h-9 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 text-sm">
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
                                <label class="text-[10px] text-[var(--ui-muted)]">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                                <select value={selected()} onChange={(e) => updateCfg(opt.id, e.currentTarget.value)} class="h-8 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 text-xs">
                                  <option value="">(default: {opt.currentValue})</option>
                                  <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                                </select>
                              </div>
                            );
                          }}
                        </For>
                        <Show when={opts.length === 0}>
                          <div class="text-[10px] text-[var(--ui-muted)] italic">
                            {configOptionsLoading() ? "Loading config options from agent..." : "No config options available for this agent."}
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
                    class="w-full resize-y rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2.5 py-2 text-xs font-mono"
                  />
                  <label class="flex items-center gap-2 text-xs text-[var(--ui-muted)]">
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

            {/* Roles list */}
            <div class="flex-1 space-y-1 overflow-auto">
              <For each={roles()}>
                {(role) => (
                  <div class="group relative flex items-start gap-1.5 rounded-lg border-l-2 border-l-transparent px-2 py-1.5 motion-safe:transition-all motion-safe:duration-150 hover:border-l-white/20 hover:bg-white/[0.04]">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-1.5 flex-wrap">
                        <span class="text-xs font-medium">{role.roleName}</span>
                        <span class={`text-xs ${RUNTIME_COLOR[role.runtimeKind] ?? "text-[var(--ui-muted)]"}`}>{role.runtimeKind}</span>
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
                      <p class="truncate text-xs text-[var(--ui-muted)]">{role.systemPrompt}</p>
                    </div>
                    <div class="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100">
                      <button
                        onClick={() => openRoleEditor(role)}
                        class={`rounded px-1.5 py-0.5 text-xs ${editingRoleId() === role.id ? "text-blue-300" : "text-[var(--ui-muted)] hover:text-[var(--ui-text)]"} ${INTERACTIVE_MOTION}`}
                      >
                        {editingRoleId() === role.id ? "editing" : "edit"}
                      </button>
                    </div>
                  </div>
                )}
              </For>
              <Show when={roles().length === 0 && !showRoleForm()}>
                <p class="rounded-lg border border-dashed border-[var(--ui-border)] p-2 text-center text-xs text-[var(--ui-muted)]">
                  No roles yet. Click "+ role" to add one.
                </p>
              </Show>
            </div>

            {/* Quick actions */}
            <div class="mt-2 flex flex-wrap gap-0.5 border-t border-white/[0.05] pt-2">
              <button onClick={() => void sendRaw("/workflow list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-[var(--ui-muted)] hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150`}>workflows</button>
              <button onClick={() => void sendRaw("/context list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-[var(--ui-muted)] hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150`}>context</button>
              <button onClick={() => void sendRaw("/session list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-[var(--ui-muted)] hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150`}>sessions</button>
            </div>
          </div>
        </aside>

        {/* ── Chat ── */}
        <main class="flex h-full flex-col overflow-hidden bg-[#09090b] p-0">

          {/* Header */}
          <div class="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--ui-border)] px-4 py-2">
            <div>
              <h1 class="text-sm font-semibold">
                <Show when={isCustomRole()} fallback="UnionAI Chat">
                  <span class="text-blue-300">{activeRole()}</span>
                  <span class="text-[var(--ui-muted)]"> @ </span>
                  <span class={RUNTIME_COLOR[selectedAssistant() ?? ""] ?? "text-[var(--ui-muted)]"}>{selectedAssistant() ?? "?"}</span>
                </Show>
              </h1>
              <p class="text-xs text-[var(--ui-muted)]">
                <Show when={!isCustomRole()}>
                  <span>Assistant: </span>
                  <span class={selectedAssistant() ? RUNTIME_COLOR[selectedAssistant()!] ?? "text-emerald-400" : "text-rose-400"}>
                    {selectedAssistant() ?? "no assistant"}
                  </span>
                </Show>
                <Show when={currentMode()}>
                  <span class={isCustomRole() ? "" : "ml-2"}>
                    <span class="rounded bg-indigo-500/20 px-1 text-[10px] text-indigo-200">{currentMode()}</span>
                  </span>
                </Show>
              </p>
            </div>
            <div class="flex items-center gap-2">
              <Show when={agentModes().length > 0}>
                <div class="flex gap-1">
                  <For each={agentModes()}>{(m) => (
                    <button
                      class={`min-h-7 rounded border px-2 py-0.5 text-xs ${INTERACTIVE_MOTION} ${currentMode() === m.id ? "border-white/25 bg-white/10 text-white" : "border-[var(--ui-border)] text-[var(--ui-muted)] hover:text-[var(--ui-text)]"}`}
                      onClick={() => {
                        const assistant = selectedAssistant();
                        const role = activeBackendRole();
                        if (assistant) void invoke("set_acp_mode", { runtimeKind: assistant, roleName: role, modeId: m.id });
                      }}
                    >
                      {m.title ?? m.id}
                    </button>
                  )}</For>
                </div>
              </Show>
              <button onClick={() => { void refreshAssistants(); void refreshRoles(); }} class={`flex h-8 w-8 items-center justify-center rounded-lg text-[var(--ui-muted)] hover:bg-white/[0.06] hover:text-white ${INTERACTIVE_MOTION}`} title="Refresh">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
              </button>
            </div>
          </div>

          {/* Messages */}
          <div id="msg-list" role="log" aria-live="polite" aria-relevant="additions text" class="flex-1 overflow-auto p-3 space-y-1.5">
            <Show when={hiddenMessageCount() > 0}>
              <div class="py-1 text-center text-xs text-[var(--ui-muted)] opacity-40">
                {hiddenMessageCount()} older messages hidden for performance
              </div>
            </Show>
            <For each={visibleMessages()}>
              {(msg) => (
                <>
                  {msg.role === "system" || msg.role === "event" ? (
                    <div class="flex items-center gap-3 px-2 py-0.5 opacity-50">
                      <div class="h-px flex-1 bg-white/[0.07]" />
                      <span class="shrink-0 text-[11px] text-[var(--ui-muted)]">{msg.text}</span>
                      <div class="h-px flex-1 bg-white/[0.07]" />
                    </div>
                  ) : (
                    <div class={`rounded-2xl border border-transparent px-3 py-2 ${msg.role === "user" ? "ml-14 bg-white/[0.07]" : "mr-14 bg-transparent"}`}>
                      <div class="mb-0.5 flex items-center justify-between text-xs text-[var(--ui-muted)]">
                        <span class="font-medium">{msg.role}</span>
                        <span>{fmt(msg.at)}</span>
                      </div>
                      <pre class="whitespace-pre-wrap break-words text-sm leading-relaxed">{msg.text}</pre>
                    </div>
                  )}
                </>
              )}
            </For>
            <Show when={streamingMessage()}>
              {(streaming) => (
                <div class="mr-14 rounded-2xl border-transparent bg-transparent px-3 py-2">
                  <div class="mb-0.5 flex items-center justify-between text-xs text-[var(--ui-muted)]">
                    <span class="font-medium">assistant</span>
                    <span>{fmt(streaming().at)}</span>
                  </div>
                  <pre class="whitespace-pre-wrap break-words text-sm leading-relaxed">{streaming().text}</pre>
                </div>
              )}
            </Show>
            <Show when={pendingPermission()}>
              {(perm) => (
                <div class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2">
                  <div class="mb-1 text-xs font-semibold text-amber-300">{perm().title}</div>
                  <Show when={perm().description}><p class="mb-2 text-xs text-[var(--ui-muted)]">{perm().description}</p></Show>
                  <div class="flex gap-2">
                    <For each={perm().options}>{(opt) => (
                      <button
                        class={`min-h-8 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
                        onClick={() => {
                          void invoke("respond_permission", { requestId: perm().requestId, optionId: opt.optionId, cancelled: false });
                          setPendingPermission(null);
                        }}
                      >
                        {opt.title ?? opt.optionId}
                      </button>
                    )}</For>
                    <button
                      class={`min-h-8 rounded border border-rose-500/40 bg-rose-500/10 px-3 py-1 text-xs text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
                      onClick={() => {
                        void invoke("respond_permission", { requestId: perm().requestId, optionId: "", cancelled: true });
                        setPendingPermission(null);
                      }}
                    >
                      Deny
                    </button>
                  </div>
                </div>
              )}
            </Show>
            <Show when={currentPlan()}>
              {(plan) => (
                <div class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel-2)] px-3 py-2">
                  <div class="mb-1 text-xs font-semibold text-[var(--ui-muted)]">Plan</div>
                  <ol class="list-inside list-decimal space-y-0.5 text-xs">
                    <For each={plan()}>{(entry) => (
                      <li class="flex items-center gap-1.5">
                        <span class={`inline-block h-1.5 w-1.5 rounded-full ${entry.status === "completed" ? "bg-emerald-400" : entry.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                        <span>{entry.title ?? entry.description ?? "step"}</span>
                      </li>
                    )}</For>
                  </ol>
                </div>
              )}
            </Show>
            <Show when={toolCalls().size > 0}>
              <div class="space-y-1">
                <For each={[...toolCalls().values()]}>{(tc) => (
                  <details class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel-2)]">
                    <summary class="flex cursor-pointer items-center gap-2 px-3 py-1.5 text-xs">
                      <span class={`h-1.5 w-1.5 rounded-full ${tc.status === "success" || tc.status === "completed" ? "bg-emerald-400" : tc.status === "failure" || tc.status === "error" ? "bg-rose-400" : tc.status === "running" || tc.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                      <span class="font-medium">{tc.title || tc.toolCallId}</span>
                      <span class="ml-auto text-[var(--ui-muted)]">{tc.status}</span>
                    </summary>
                    <Show when={tc.contentJson}>
                      <pre class="whitespace-pre-wrap break-words border-t border-[var(--ui-border)] px-3 py-1.5 text-xs text-[var(--ui-muted)]">
                        {tc.contentJson}
                      </pre>
                    </Show>
                  </details>
                )}</For>
              </div>
            </Show>
            <Show when={submitting()}>
              <div class="flex items-center gap-2 px-1 text-xs text-[var(--ui-muted)] opacity-80">
                <span class="h-2 w-2 rounded-full bg-white/60 animate-pulse" />
                <span>Agent is thinking...</span>
              </div>
            </Show>
          </div>

          {/* Input */}
          <form onSubmit={handleSend} class="shrink-0 border-t border-white/[0.05] p-3">
            <div class="relative">
              <div class="flex items-center rounded-2xl border border-white/[0.08] bg-white/[0.03] px-3 gap-1.5 focus-within:border-white/[0.14] motion-safe:transition-colors">
                <button
                  type="button"
                  onClick={() => setActiveRole(DEFAULT_ROLE_ALIAS)}
                  class={`shrink-0 py-2.5 text-sm font-mono ${isCustomRole() ? "text-blue-300 hover:text-blue-200" : "text-[var(--ui-muted)]"}`}
                  title={isCustomRole() ? "Click to return to UnionAI" : "UnionAI mode"}
                >
                  {activeRole()} &gt;
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
                  placeholder={isCustomRole() ? `Chat with ${activeRole()}... (type / for agent commands)` : "Natural language / commands / @role @file:path"}
                  class="flex-1 bg-transparent py-2.5 text-sm outline-none min-w-0"
                />
                <button
                  type="submit"
                  class={`shrink-0 flex h-7 w-7 items-center justify-center rounded-xl motion-safe:transition-all motion-safe:duration-150 ${input().trim() ? "bg-white text-zinc-950" : "bg-white/[0.12] text-white/30"} ${INTERACTIVE_MOTION}`}
                  title={submitting() ? "Queue" : "Send"}
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>
                </button>
              </div>
              <Show when={slashOpen() && slashItems().length > 0}>
                <div class="absolute bottom-12 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel)] p-1 shadow-xl">
                  <For each={slashItems()}>
                    {(item, i) => (
                      <button
                        type="button"
                        onMouseDown={(e) => {
                          e.preventDefault();
                          applySlashCandidate(item);
                        }}
                        class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === slashActiveIndex() ? "bg-[var(--ui-accent-soft)] text-[var(--ui-text)]" : "text-[var(--ui-muted)] hover:bg-[var(--ui-panel-2)]"}`}
                      >
                        <span class="rounded bg-indigo-500/20 px-1 text-[10px] uppercase tracking-wide text-indigo-200">
                          cmd
                        </span>
                        <span class="truncate font-mono text-xs">{item.value}</span>
                        <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <Show when={mentionOpen() && mentionItems().length > 0}>
                <div class="absolute bottom-12 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel)] p-1 shadow-xl">
                  <For each={mentionItems()}>
                    {(item, i) => (
                      <button
                        type="button"
                        onMouseDown={(e) => {
                          e.preventDefault();
                          applyMentionCandidate(item);
                        }}
                        class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === mentionActiveIndex() ? "bg-[var(--ui-accent-soft)] text-[var(--ui-text)]" : "text-[var(--ui-muted)] hover:bg-[var(--ui-panel-2)]"}`}
                      >
                        <span class={`rounded px-1 text-[10px] uppercase tracking-wide ${item.kind === "role" ? "bg-blue-500/20 text-blue-200" : item.kind === "dir" ? "bg-emerald-500/20 text-emerald-200" : item.kind === "file" ? "bg-amber-500/20 text-amber-200" : item.kind === "command" ? "bg-indigo-500/20 text-indigo-200" : "bg-zinc-500/20 text-zinc-300"}`}>
                          {item.kind}
                        </span>
                        <span class="truncate font-mono text-xs">{item.value}</span>
                        <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </form>
        </main>

      </div>
    </div>
  );
}
