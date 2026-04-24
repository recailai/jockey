import { openUrl } from "@tauri-apps/plugin-opener";
import { For, Index, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, AppMessage, AppToolCall, AppSegment } from "./types";
import { RUNTIME_COLOR, MESSAGE_RENDER_WINDOW, fmt } from "./types";
import { identicon } from "../lib/identicon";
import { renderMd, renderMdCached } from "../lib/markdown";
import { ToolCallGroup } from "./ToolCallGroup";
import { PermissionModal } from "./PermissionModal";
import SessionErrorBanner from "./SessionErrorBanner";

function highlightText(text: string, query: string): string {
  if (!query) return text;
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.replace(new RegExp(escaped, "gi"), (m) => `<mark class="search-highlight">${m}</mark>`);
}

type MessageWindowProps = {
  activeSessionId: Accessor<string | null>;
  activeSession: Accessor<AppSession | null>;
  activeBackendRole: () => string;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  onResetAgentContext?: () => void;
  onReconnectAgent?: () => void;
  onListMounted?: (id: string, el: HTMLElement) => void;
  onListUnmounted?: (id: string) => void;
};

export default function MessageWindow(props: MessageWindowProps) {
  let listEl: HTMLDivElement | undefined;
  let searchInputEl: HTMLInputElement | undefined;
  let boundSessionId: string | null = null;
  const [queueCollapsed, setQueueCollapsed] = createSignal(true);
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [searchQuery, setSearchQuery] = createSignal("");

  createEffect(() => {
    const id = props.activeSessionId();
    if (!listEl || !id) return;
    if (boundSessionId && boundSessionId !== id) props.onListUnmounted?.(boundSessionId);
    props.onListMounted?.(id, listEl);
    boundSessionId = id;
  });

  onCleanup(() => {
    if (boundSessionId) props.onListUnmounted?.(boundSessionId);
  });

  type VisibleMsg = { msg: AppMessage; count: number; latestAt: number };

  const isGroupable = (m: AppMessage) => m.roleName === "system" || m.roleName === "event";

  // Incremental dedup: only re-process rows appended since last run.
  // Reset state is tied to session id — session switch always triggers full rebuild.
  let _prevRows: readonly AppMessage[] = [];
  let _prevSessionId: string | null = null;

  const visibleMessages = createMemo<VisibleMsg[]>((prev) => {
    const sessionId = props.activeSessionId();
    const rows = props.activeSession()?.messages ?? [];
    const sliceStart = rows.length > MESSAGE_RENDER_WINDOW ? rows.length - MESSAGE_RENDER_WINDOW : 0;

    // Fast path: same session, new messages only appended at the tail
    const prevRows = _prevRows;
    const prevDeduped = prev ?? [];
    const isAppendOnly =
      sessionId === _prevSessionId &&
      prevRows.length > 0 &&
      rows.length > prevRows.length &&
      rows[prevRows.length - 1] === prevRows[prevRows.length - 1];

    let deduped: VisibleMsg[];
    let newStart: number;

    if (isAppendOnly && sliceStart <= prevRows.length) {
      deduped = [...prevDeduped];
      newStart = prevRows.length;
    } else {
      deduped = [];
      newStart = sliceStart;
    }

    for (let i = newStart; i < rows.length; i++) {
      const msg = rows[i];
      if (isGroupable(msg)) {
        const last = deduped[deduped.length - 1];
        if (last && isGroupable(last.msg) && last.msg.text === msg.text) {
          deduped[deduped.length - 1] = { ...last, count: last.count + 1, latestAt: msg.at };
          continue;
        }
      }
      deduped.push({ msg, count: 1, latestAt: msg.at });
    }

    _prevRows = rows;
    _prevSessionId = sessionId;
    return deduped;
  }, []);

  const hiddenMessageCount = (): number => {
    const count = (props.activeSession()?.messages.length ?? 0) - MESSAGE_RENDER_WINDOW;
    return count > 0 ? count : 0;
  };

  const filteredMessages = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return visibleMessages();
    return visibleMessages().filter((item) =>
      item.msg.text?.toLowerCase().includes(q)
    );
  });

  const closeSearch = () => {
    setSearchOpen(false);
    setSearchQuery("");
  };

  function handleContainerClick(e: MouseEvent) {
    const target = e.target as HTMLElement;

    const copyBtn = target.closest("[data-copy-code]");
    if (copyBtn) {
      e.preventDefault();
      const pre = copyBtn.closest("pre");
      if (pre) {
        const code = pre.querySelector("code");
        const text = (code ?? pre).textContent ?? "";
        navigator.clipboard.writeText(text).then(() => {
          copyBtn.textContent = "✓";
          setTimeout(() => { copyBtn.textContent = "Copy"; }, 1500);
        });
      }
      return;
    }

    const a = target.closest("a");
    if (!a) return;
    const href = a.getAttribute("href");
    if (!href || !href.startsWith("http")) return;
    e.preventDefault();
    openUrl(href);
  }

  const [ctxMenu, setCtxMenu] = createSignal<{ x: number; y: number } | null>(null);

  function handleResetContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ x: e.clientX, y: e.clientY });
  }

  function closeCtxMenu() {
    setCtxMenu(null);
  }

  onMount(() => {
    window.addEventListener("click", closeCtxMenu);

    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        setSearchOpen(true);
        window.requestAnimationFrame(() => searchInputEl?.focus());
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);

    onCleanup(() => {
      window.removeEventListener("click", closeCtxMenu);
      window.removeEventListener("keydown", handleGlobalKeyDown);
    });
  });

  return (
    <div class="flex-1 flex flex-col min-h-0 relative" style={{ "background-color": "transparent" }}>
      <Show when={searchOpen()}>
        <div class="absolute top-2 right-3 z-30 flex items-center gap-1.5 rounded-xl border theme-border theme-panel shadow-lg backdrop-blur-md px-2 py-1.5">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 theme-muted">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          <input
            ref={searchInputEl}
            type="text"
            placeholder="Search messages…"
            class="bg-transparent outline-none text-[12px] theme-text placeholder:theme-muted w-48"
            value={searchQuery()}
            onInput={(e) => setSearchQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") { e.preventDefault(); closeSearch(); }
            }}
          />
          <Show when={searchQuery()}>
            <span class="text-[10px] theme-muted font-mono shrink-0">{filteredMessages().length} / {visibleMessages().length}</span>
          </Show>
          <button
            class="ml-1 theme-muted hover:theme-text transition-colors"
            onClick={closeSearch}
          >
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      </Show>
    <div
      ref={listEl}
      id={`msg-list-${props.activeSessionId()}`}
      role="log"
      aria-live="polite"
      aria-relevant="additions text"
      class="flex-1 overflow-auto px-4 py-4 space-y-4"
      style={{ "background-color": "transparent" }}
      onClick={handleContainerClick}
    >
      <Show when={hiddenMessageCount() > 0}>
        <div class="py-1 text-center text-xs theme-muted opacity-40">
          {hiddenMessageCount()} older messages hidden for performance
        </div>
      </Show>
      <Show when={searchQuery() && filteredMessages().length === 0}>
        <div class="py-8 text-center text-xs theme-muted opacity-60">No messages match "{searchQuery()}"</div>
      </Show>
      <For each={filteredMessages()}>
        {(item) => {
          const msg = item.msg;
          const q = searchQuery();
          if (msg.roleName === "user") return (
            <div class="flex flex-col items-end w-full mb-3 group/user">
              <div class="user-bubble max-w-[85%] rounded-2xl rounded-tr-md px-4 py-2 text-[13px]">
                <Show when={q} fallback={
                  <div class="whitespace-pre-wrap break-words leading-relaxed font-mono">{msg.text}</div>
                }>
                  <div class="whitespace-pre-wrap break-words leading-relaxed font-mono" innerHTML={highlightText(msg.text, q)} />
                </Show>
              </div>
              <div class="mt-1.5 text-[10px] theme-muted mr-1 opacity-0 transition-opacity duration-300 group-hover/user:opacity-100 tracking-wide">{fmt(msg.at)}</div>
            </div>
          );
          if (msg.roleName === "system" || msg.roleName === "event") return (
            <div class="flex justify-center my-3 relative">
              <div class="absolute inset-x-0 top-1/2 h-px bg-gradient-to-r from-transparent via-[var(--ui-border)] to-transparent -z-10"></div>
              <div class="max-w-[90%] px-4 py-1.5 border rounded-full text-[11.5px] flex items-center gap-2.5 backdrop-blur-md shadow-sm theme-panel theme-border theme-muted">
                <span class="opacity-70 text-indigo-400 mt-[1px] font-serif">✧</span>
                <Show when={q} fallback={
                  <span class="whitespace-pre-wrap break-words tracking-wide">{msg.text}</span>
                }>
                  <span class="whitespace-pre-wrap break-words tracking-wide" innerHTML={highlightText(msg.text, q)} />
                </Show>
                <Show when={item.count > 1}>
                  <span class="bg-indigo-500/15 border border-indigo-500/20 text-indigo-300 font-mono rounded-full px-1.5 py-0.5 text-[9px] min-w-[20px] text-center">{item.count}</span>
                </Show>
              </div>
            </div>
          );
          const hasContent = !!msg.text?.trim() || (msg.segments && msg.segments.length > 0) || (msg.toolCalls && msg.toolCalls.length > 0);
          if (!hasContent) return null;
          return (
            <div class="flex gap-4 w-full max-w-[95%] mb-6 group/agent">
              <button
                type="button"
                onContextMenu={handleResetContextMenu}
                class="relative h-8 w-8 shrink-0 rounded-xl border hover:border-indigo-400/60 transition-colors flex items-center justify-center shadow-lg ring-1 ring-black/20 mt-0.5 overflow-hidden cursor-pointer theme-border"
                title="Right-click to reset current agent CLI context"
                innerHTML={identicon(msg.roleName)}
              />
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2.5 mb-1.5 opacity-90">
                  <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.runtimeKind ?? ""] ?? "text-zinc-300"}`}>
                    {msg.roleName}
                  </span>
                  <span class="text-[10px] theme-muted font-medium">{fmt(msg.at)}</span>
                  <Show when={props.activeSession()?.currentMode}>
                    <span class="rounded-md bg-indigo-500/15 border border-indigo-500/30 px-2 py-0.5 text-[9px] font-semibold text-indigo-300 uppercase tracking-wider">{props.activeSession()?.currentMode}</span>
                  </Show>
                </div>
                <Show when={msg.segments && msg.segments.length > 0} fallback={
                  <div class="md-prose" innerHTML={q ? highlightText(renderMdCached(msg.id, msg.text), q) : renderMdCached(msg.id, msg.text)} />
                }>
                  <SegmentList segments={msg.segments!} terminals={props.activeSession()?.terminals} />
                </Show>
              </div>
            </div>
          );
        }}
      </For>
      <Show when={props.activeSession()?.streamingMessage}>
        {(streaming) => (
          <div class="flex gap-4 w-full max-w-[95%] mb-6">
            <button
              type="button"
              onContextMenu={handleResetContextMenu}
              class="relative h-8 w-8 shrink-0 rounded-xl border border-indigo-500/30 transition-colors flex items-center justify-center shadow-[0_0_16px_rgba(99,102,241,0.3)] ring-1 ring-indigo-500/20 mt-0.5 overflow-hidden cursor-pointer"
              style={{ background: "radial-gradient(circle at 50% 50%, rgba(99,102,241,0.15), transparent 70%)" }}
              title="Right-click to reset current agent CLI context"
            >
              <svg class="absolute inset-0 h-full w-full" viewBox="0 0 32 32" fill="none">
                <circle cx="16" cy="16" r="13" stroke="rgba(99,102,241,0.1)" stroke-width="0.5" />
                <circle cx="16" cy="16" r="10" stroke="rgba(99,102,241,0.08)" stroke-width="0.5" />
                <path d="M16 3a13 13 0 0 1 13 13" stroke="url(#arc1)" stroke-width="1.5" stroke-linecap="round" class="origin-center" style={{ animation: "spin 1.5s linear infinite" }} />
                <path d="M16 5a11 11 0 0 0-11 11" stroke="url(#arc2)" stroke-width="1" stroke-linecap="round" class="origin-center" style={{ animation: "spin 2.5s linear infinite reverse" }} />
                <path d="M16 7a9 9 0 0 1 9 9" stroke="url(#arc3)" stroke-width="0.8" stroke-linecap="round" class="origin-center" style={{ animation: "spin 3.5s linear infinite" }} />
                <circle cx="16" cy="16" r="2.5" fill="url(#core)" class="animate-pulse" />
                <circle cx="16" cy="16" r="4" stroke="rgba(129,140,248,0.3)" stroke-width="0.5" class="animate-pulse" />
                <defs>
                  <linearGradient id="arc1"><stop stop-color="#818cf8" /><stop offset="1" stop-color="#818cf8" stop-opacity="0" /></linearGradient>
                  <linearGradient id="arc2"><stop stop-color="#c084fc" /><stop offset="1" stop-color="#c084fc" stop-opacity="0" /></linearGradient>
                  <linearGradient id="arc3"><stop stop-color="#22d3ee" /><stop offset="1" stop-color="#22d3ee" stop-opacity="0" /></linearGradient>
                  <radialGradient id="core"><stop stop-color="#a5b4fc" /><stop offset="1" stop-color="#6366f1" /></radialGradient>
                </defs>
              </svg>
            </button>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2.5 mb-2">
                <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.runtimeKind ?? ""] ?? "text-zinc-300"}`}>
                  {props.activeSession()?.activeRole ?? "Agent"}
                </span>
              </div>
              <Show when={(props.activeSession()?.streamSegments ?? []).length > 0} fallback={
                <>
                  <Show when={streaming().text}>
                    <div class="md-prose" innerHTML={renderMd(streaming().text)} />
                  </Show>
                  <Show when={!streaming().text && props.activeSession()?.agentState}>
                    <div class="text-[11px] theme-muted italic">{props.activeSession()?.agentState}</div>
                  </Show>
                </>
              }>
                <StreamSegmentList
                  segments={props.activeSession()?.streamSegments ?? []}
                  terminals={props.activeSession()?.terminals}
                />
              </Show>
              <Show when={props.activeSession()?.thoughtText}>
                <div class="mt-2 rounded-md border theme-border theme-panel px-2.5 py-2 text-[11px] theme-muted leading-relaxed font-mono whitespace-pre-wrap break-words">
                  {props.activeSession()?.thoughtText}
                </div>
              </Show>
              <div class="flex items-center gap-2 mt-2 pt-1.5">
                <svg class="h-3 w-3 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
                  <path d="M12 2a10 10 0 0 1 7.07 2.93" stroke="var(--ui-accent, #818cf8)" /><path d="M22 12a10 10 0 0 1-2.93 7.07" stroke="var(--ui-accent, #818cf8)" opacity="0.6" /><path d="M12 22a10 10 0 0 1-7.07-2.93" stroke="var(--ui-accent, #818cf8)" opacity="0.3" /><path d="M2 12a10 10 0 0 1 2.93-7.07" stroke="var(--ui-accent, #818cf8)" opacity="0.1" />
                </svg>
                <span class="text-[10px] theme-muted italic">{props.activeSession()?.agentState || "running"}</span>
              </div>
            </div>
          </div>
        )}
      </Show>
      <SessionErrorBanner
        activeSession={props.activeSession}
        activeSessionId={props.activeSessionId}
        activeBackendRole={props.activeBackendRole}
        patchActiveSession={props.patchActiveSession}
      />
      <PermissionModal
        activeSession={props.activeSession}
        patchActiveSession={props.patchActiveSession}
      />
      <Show when={props.activeSession()?.currentPlan}>
        {(plan) => (
          <div class="rounded-lg border theme-border theme-surface px-3 py-2 my-2">
            <div class="mb-1 text-xs font-semibold theme-muted">Plan</div>
            <ol class="list-inside list-decimal space-y-0.5 text-xs">
              <For each={plan()}>{(entry) => (
                <li class="flex items-center gap-1.5">
                  <span class={`inline-block h-1.5 w-1.5 rounded-full ${entry.status === "completed" ? "bg-emerald-400" : entry.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                  <span class="theme-text">{entry.content ?? entry.title ?? entry.description ?? "step"}</span>
                </li>
              )}</For>
            </ol>
          </div>
        )}
      </Show>
      <Show when={props.activeSession()?.submitting && !props.activeSession()?.streamingMessage}>
        <div class="flex items-center gap-2 px-1 text-xs theme-muted opacity-80 mt-2">
          <span class="h-2 w-2 rounded-full bg-white/60 animate-pulse" />
          <span>{props.activeSession()?.agentState || "Agent is thinking..."}</span>
        </div>
      </Show>
      <Show when={(props.activeSession()?.queuedMessages ?? []).length > 0}>
        <div class="mt-3 rounded-lg border theme-border backdrop-blur-sm overflow-hidden theme-panel">
          <button
            class="flex w-full items-center gap-2 px-3 py-1.5 hover:bg-[var(--ui-accent-soft)] transition-colors"
            classList={{ "border-b theme-border": !queueCollapsed() }}
            onClick={() => setQueueCollapsed(v => !v)}
          >
            <span class="h-1.5 w-1.5 rounded-full bg-indigo-400 animate-pulse shrink-0" />
            <span class="text-[10px] theme-muted font-medium uppercase tracking-wider">Queued</span>
            <span class="ml-auto text-[9px] theme-muted font-mono bg-[var(--ui-panel-2)] px-1.5 py-0.5 rounded-md">{props.activeSession()!.queuedMessages.length}</span>
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class="theme-muted transition-transform ml-1" classList={{ "rotate-180": !queueCollapsed() }}>
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>
          <Show when={!queueCollapsed()}>
            <div class="px-2 py-1.5 space-y-1">
              <For each={props.activeSession()!.queuedMessages}>{(text, i) => (
                <div class="flex items-start gap-2 px-2 py-1 rounded-md hover:bg-[var(--ui-accent-soft)] transition-colors">
                  <span class="text-[9px] theme-muted font-mono mt-0.5 shrink-0 w-4 text-right">{i() + 1}</span>
                  <span class="text-[11px] theme-text font-mono break-all leading-relaxed">{text}</span>
                </div>
              )}</For>
            </div>
          </Show>
        </div>
      </Show>
      <Show when={ctxMenu()}>
        {(pos) => (
          <div
            class="fixed z-[200] min-w-[140px] overflow-hidden rounded-lg shadow-xl shadow-black/60 backdrop-blur-md py-1 theme-dropdown"
            style={`left:${pos().x}px;top:${pos().y}px`}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] theme-text hover:bg-[var(--ui-accent-soft)] transition-colors"
              onClick={() => { closeCtxMenu(); props.onResetAgentContext?.(); }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 text-zinc-500">
                <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>
              </svg>
              Reset context
            </button>
            <button
              class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] theme-text hover:bg-[var(--ui-accent-soft)] transition-colors"
              onClick={() => { closeCtxMenu(); props.onReconnectAgent?.(); }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 text-zinc-500">
                <path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>
              </svg>
              Reconnect agent
            </button>
          </div>
        )}
      </Show>
    </div>
    </div>
  );
}

function collectToolGroups(segments: AppSegment[]): Array<{ kind: "text"; text: string } | { kind: "tools"; tools: AppToolCall[] }> {
  const result: Array<{ kind: "text"; text: string } | { kind: "tools"; tools: AppToolCall[] }> = [];
  for (const seg of segments) {
    if (seg.kind === "text") {
      result.push(seg);
    } else {
      const last = result[result.length - 1];
      if (last && last.kind === "tools") {
        last.tools.push(seg.tc);
      } else {
        result.push({ kind: "tools", tools: [seg.tc] });
      }
    }
  }
  return result;
}

function SegmentList(props: { segments: AppSegment[]; terminals?: AppSession["terminals"] }) {
  const groups = createMemo(() => collectToolGroups(props.segments));
  return (
    <For each={groups()}>{(g) => (
      g.kind === "text"
        ? <div class="md-prose" innerHTML={renderMd(g.text)} />
        : <ToolCallGroup tools={g.tools} streaming={false} terminals={props.terminals} />
    )}</For>
  );
}

function StreamSegmentList(props: { segments: AppSegment[]; terminals?: AppSession["terminals"] }) {
  const groups = createMemo(() => collectToolGroups(props.segments));
  return (
    <Index each={groups()}>{(g) => (
      <Switch>
        <Match when={g().kind === "text"}>
          <div class="md-prose" innerHTML={renderMd((g() as { kind: "text"; text: string }).text)} />
        </Match>
        <Match when={g().kind === "tools"}>
          <ToolCallGroup
            tools={(g() as { kind: "tools"; tools: AppToolCall[] }).tools}
            streaming={true}
            terminals={props.terminals}
          />
        </Match>
      </Switch>
    )}</Index>
  );
}

