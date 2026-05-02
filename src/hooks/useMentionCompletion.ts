import { createSignal } from "solid-js";
import type { AppMentionItem, AppSkill, Role } from "../components/types";
import { MENTION_CACHE_LIMIT } from "../lib/sessionHelpers";
import { completionApi } from "../lib/tauriApi";

export function useMentionCompletion(
  input: () => string,
  setInput: (v: string) => void,
  getInputEl: () => HTMLInputElement | undefined,
  activeSessionId: () => string | null,
  roles: () => Role[],
  skills: () => AppSkill[],
) {
  const [mentionOpen, setMentionOpen] = createSignal(false);
  const [mentionItems, setMentionItems] = createSignal<AppMentionItem[]>([]);
  const [mentionActiveIndex, setMentionActiveIndex] = createSignal(0);
  const [mentionRange, setMentionRange] = createSignal<{ start: number; end: number; query: string } | null>(null);

  let mentionReqSeq = 0;
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
    const roleItems = listRoleMentionCandidates(ctx.query);
    let items: AppMentionItem[] = [...roleItems];
    let pathRows: AppMentionItem[] = [];

    if (shouldPathComplete(ctx.query)) {
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

    if (merged.length === 0) {
      closeMentionMenu();
      return;
    }
    setMentionItems(merged);
    setMentionActiveIndex(0);
    setMentionOpen(true);
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

  return {
    mentionOpen,
    mentionItems,
    mentionActiveIndex,
    mentionRange,
    mentionCloseTimerRef,
    mentionDebounceTimerRef,
    closeMentionMenu,
    refreshMentionSuggestions,
    applyMentionCandidate,
    extractMentionContext,
    _setMentionActiveIndex: setMentionActiveIndex,
  };
}

export type MentionCompletion = ReturnType<typeof useMentionCompletion>;
