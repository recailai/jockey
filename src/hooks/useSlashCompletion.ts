import { createSignal } from "solid-js";
import type { AppMentionItem, AcpConfigOption, AppSession, Role } from "../components/types";
import { flattenConfigValues } from "../components/types";
import { completionApi } from "../lib/tauriApi";

export function useSlashCompletion(
  input: () => string,
  setInput: (v: string) => void,
  getInputEl: () => HTMLInputElement | undefined,
  activeSessionId: () => string | null,
  activeSession: () => AppSession | null,
  patchActiveSession: (patch: Partial<AppSession>) => void,
  roles: () => Role[],
  isCustomRole: () => boolean,
  normalizeRuntimeKey: (runtimeKey: string) => string,
  commandCacheKey: (runtimeKey: string, roleName: string) => string,
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>,
  hydrateAgentCommandsForSession: (sessionId: string, runtimeKey: string, roleName: string) => Promise<number>,
  slashCliCacheRef: { cache: null | unknown[]; version: number },
) {
  const [slashOpen, setSlashOpen] = createSignal(false);
  const [slashItems, setSlashItems] = createSignal<AppMentionItem[]>([]);
  const [slashActiveIndex, setSlashActiveIndex] = createSignal(0);
  const [slashRange, setSlashRange] = createSignal<{ end: number; query: string } | null>(null);

  let slashReqSeq = 0;

  const closeSlashMenu = () => {
    setSlashOpen(false);
    setSlashItems([]);
    setSlashActiveIndex(0);
    setSlashRange(null);
  };

  const extractSlashContext = (text: string, caret: number) => {
    const left = text.slice(0, caret);
    if (!left.startsWith("/")) return null;
    if (left.includes("\n")) return null;
    return { end: caret, query: left.trimEnd() };
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

  return {
    slashOpen,
    slashItems,
    slashActiveIndex,
    slashRange,
    closeSlashMenu,
    refreshSlashSuggestions,
    applySlashCandidate,
    extractSlashContext,
    _setSlashActiveIndex: setSlashActiveIndex,
  };
}

export type SlashCompletion = ReturnType<typeof useSlashCompletion>;
