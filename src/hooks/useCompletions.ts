import { createSignal } from "solid-js";
import type { AppMentionItem } from "../components/types";
import { flattenConfigValues } from "../components/types";
import { MENTION_CACHE_LIMIT } from "../lib/sessionHelpers";
import type { AgentContext } from "./useAgentContext";
import type { SessionManager } from "./useSessionManager";
import { completionApi } from "../lib/tauriApi";

export function useCompletions(
  agentContext: AgentContext,
  sessionManager: SessionManager,
  input: () => string,
  setInput: (v: string) => void,
  getInputEl: () => HTMLInputElement | undefined,
) {
  const {
    roles,
    skills,
    normalizeRuntimeKey,
    commandCacheKey,
    isCustomRole,
    fetchConfigOptions,
    hydrateAgentCommandsForSession,
    slashCliCacheRef,
  } = agentContext;

  const {
    activeSessionId,
    activeSession,
    patchActiveSession,
    sessions,
  } = sessionManager;

  const [mentionOpen, setMentionOpen] = createSignal(false);
  const [mentionItems, setMentionItems] = createSignal<AppMentionItem[]>([]);
  const [mentionActiveIndex, setMentionActiveIndex] = createSignal(0);
  const [mentionRange, setMentionRange] = createSignal<{ start: number; end: number; query: string } | null>(null);
  const [slashOpen, setSlashOpen] = createSignal(false);
  const [slashItems, setSlashItems] = createSignal<AppMentionItem[]>([]);
  const [slashActiveIndex, setSlashActiveIndex] = createSignal(0);
  const [slashRange, setSlashRange] = createSignal<{ end: number; query: string } | null>(null);

  let mentionReqSeq = 0;
  let slashReqSeq = 0;
  const mentionCloseTimerRef = { current: null as number | null };
  const mentionDebounceTimerRef = { current: null as number | null };
  let mentionPathCache = new Map<string, AppMentionItem[]>();
  let mentionPathCacheKeys: string[] = [];
  let mentionPathCacheOwner: string | null = null;

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
    const roleItems = listRoleMentionCandidates(ctx.query);
    let items: AppMentionItem[] = [...staticItems, ...roleItems];
    let pathAttempted = false;
    let pathRows: AppMentionItem[] = [];

    if (shouldPathComplete(ctx.query)) {
      pathAttempted = true;
      const seq = ++mentionReqSeq;
      const sid = activeSessionId() ?? null;
      if (mentionPathCacheOwner !== sid) {
        mentionPathCache = new Map();
        mentionPathCacheKeys = [];
        mentionPathCacheOwner = sid;
      }
      const cached = mentionPathCache.get(ctx.query);
      if (cached) {
        pathRows = cached;
      } else {
        try {
          const rows = await completionApi.mentions(ctx.query, 12, sid);
          if (seq !== mentionReqSeq) return;
          mentionPathCache.set(ctx.query, rows);
          mentionPathCacheKeys.push(ctx.query);
          if (mentionPathCacheKeys.length > MENTION_CACHE_LIMIT) {
            const evictKey = mentionPathCacheKeys.shift()!;
            mentionPathCache.delete(evictKey);
          }
          pathRows = rows;
        } catch {
          pathRows = [];
        }
      }
      items = [...items, ...pathRows];
    }

    const dedup = new Set<string>();
    const merged = items.filter((it) => {
      const key = `${it.kind}:${it.value}`;
      if (dedup.has(key)) return false;
      dedup.add(key);
      return true;
    }).slice(0, 12);

    // If the query looks like a path lookup but nothing matched on disk,
    // don't let static hints/role items keep the menu open — close it.
    const nonHintCount = merged.filter((it) => it.kind !== "hint").length;
    if (merged.length === 0 || (pathAttempted && ctx.query.length > 0 && pathRows.length === 0 && nonHintCount === 0)) {
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
      const roleName = s?.activeRole;
      if (!roleName) { closeSlashMenu(); return; }
      const runtimeRaw = roles().find((r) => r.roleName === roleName)?.runtimeKind ?? s?.runtimeKind ?? "";
      const runtimeKey = normalizeRuntimeKey(runtimeRaw);
      if (!runtimeKey) { closeSlashMenu(); return; }
      const key = commandCacheKey(runtimeKey, roleName);
      if ((s?.discoveredConfigOptions.length ?? 0) === 0) {
        const opts = await fetchConfigOptions(runtimeKey, roleName);
        if (seq !== slashReqSeq) return;
        patchActiveSession({ discoveredConfigOptions: opts });
      }
      if ((s?.agentCommands.get(key) ?? []).length === 0) {
        const sid = activeSessionId();
        if (sid) await hydrateAgentCommandsForSession(sid, runtimeKey, roleName);
      }
      const candidates = buildAgentSlashCandidates(runtimeKey, roleName, ctx.query);
      if (seq !== slashReqSeq) return;
      if (candidates.length === 0) { closeSlashMenu(); return; }
      setSlashItems(candidates);
      setSlashActiveIndex(0);
      setSlashOpen(true);
      return;
    }

    try {
      const version = slashCliCacheRef.version;
      const all = (slashCliCacheRef.cache as AppMentionItem[] | null) ?? await (async () => {
        const rows = await completionApi.cli("", 200);
        if (slashCliCacheRef.version === version) {
          slashCliCacheRef.cache = rows;
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
    const target = getInputEl();
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
    const target = getInputEl();
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

  // suppress unused warning — sessions is read inside refreshSlashSuggestions via activeSession
  void sessions;

  return {
    mentionOpen,
    mentionItems,
    mentionActiveIndex,
    mentionRange,
    slashOpen,
    slashItems,
    slashActiveIndex,
    slashRange,
    mentionCloseTimerRef,
    mentionDebounceTimerRef,
    closeMentionMenu,
    closeSlashMenu,
    refreshInputCompletions,
    applyMentionCandidate,
    applySlashCandidate,
    _setSlashActiveIndex: setSlashActiveIndex,
    _setMentionActiveIndex: setMentionActiveIndex,
  };
}

export type Completions = ReturnType<typeof useCompletions>;
