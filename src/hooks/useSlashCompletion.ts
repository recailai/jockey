import { createSignal } from "solid-js";
import type { AppMentionItem, AcpConfigOption, AppSession, Role } from "../components/types";

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
  onTriggerCommandUi?: (commandName: string) => boolean,
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
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    const s = activeSession();
    let cmds = s?.agentCommands.get(commandCacheKey(normalizedRuntime, roleName)) ?? [];
    if (cmds.length === 0) {
      const altRuntime =
        normalizedRuntime === "claude-code"
          ? "claude-native"
          : normalizedRuntime === "claude-native"
          ? "claude-code"
          : "";
      if (altRuntime) {
        cmds = s?.agentCommands.get(commandCacheKey(altRuntime, roleName)) ?? [];
      }
    }

    const seen = new Set<string>();
    const out: AppMentionItem[] = [];
    for (const cmd of cmds) {
      if (seen.has(cmd.name)) continue;
      seen.add(cmd.name);
      const nameLower = cmd.name.toLowerCase();
      if (queryLower && !nameLower.includes(queryLower)) continue;
      out.push({
        value: `/${cmd.name}`,
        kind: (cmd as { kind?: "command" | "skill" }).kind === "skill" ? "skill" : "command",
        detail: cmd.description,
        source: "agent",
      });
    }

    const clientContributions = [
      { name: "clear", description: "Reset active conversation context" },
      { name: "context", description: "Toggle workspace files & git context panel" },
      { name: "reset", description: "Reset active conversation context" },
    ];
    for (const contrib of clientContributions) {
      if (seen.has(contrib.name)) continue;
      seen.add(contrib.name);
      if (queryLower && !contrib.name.includes(queryLower)) continue;
      out.push({
        value: `/${contrib.name}`,
        kind: "command",
        detail: contrib.description,
        source: "client",
      });
    }

    if (queryLower) {
      out.sort((a, b) => {
        const aName = a.value.slice(1).toLowerCase();
        const bName = b.value.slice(1).toLowerCase();
        const aExact = aName === queryLower ? 0 : 1;
        const bExact = bName === queryLower ? 0 : 1;
        if (aExact !== bExact) return aExact - bExact;

        const aPrefix = aName.startsWith(queryLower) ? 0 : 1;
        const bPrefix = bName.startsWith(queryLower) ? 0 : 1;
        if (aPrefix !== bPrefix) return aPrefix - bPrefix;

        return aName.localeCompare(bName);
      });
    }

    return out.slice(0, 40);
  };

  const refreshSlashSuggestions = async (text: string, caret: number) => {
    const ctx = extractSlashContext(text, caret);
    if (!ctx) {
      closeSlashMenu();
      return;
    }
    const prevQuery = slashRange()?.query;
    setSlashRange(ctx);
    const seq = ++slashReqSeq;

    // The session's runtime is authoritative: a persona's own runtime_kind is only a seed,
    // and the project may have pinned a different engine for this persona.
    const s = activeSession();
    const roleName = s?.activeRole || "Developer";
    const runtimeKey = normalizeRuntimeKey(
      s?.runtimeKind ?? roles().find((r) => r.roleName === roleName)?.runtimeKind ?? "",
    );

    let agentItems: AppMentionItem[] = [];
    if (runtimeKey) {
      const key = commandCacheKey(runtimeKey, roleName);
      if ((s?.agentCommands.get(key) ?? []).length === 0) {
        const sid = activeSessionId();
        if (sid) await hydrateAgentCommandsForSession(sid, runtimeKey, roleName);
      }
      agentItems = buildAgentSlashCandidates(runtimeKey, roleName, ctx.query);
    }
    if (seq !== slashReqSeq) return;

    if (agentItems.length === 0) {
      closeSlashMenu();
      return;
    }
    setSlashItems(agentItems);
    if (prevQuery === ctx.query) {
      setSlashActiveIndex((idx) => Math.min(Math.max(idx, 0), Math.max(0, agentItems.length - 1)));
    } else {
      setSlashActiveIndex(0);
    }
    setSlashOpen(true);
  };

  const applySlashCandidate = (item: AppMentionItem) => {
    const commandName = item.value.replace(/^\//, "");
    if (onTriggerCommandUi && onTriggerCommandUi(commandName)) {
      closeSlashMenu();
      setInput("");
      return;
    }
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
