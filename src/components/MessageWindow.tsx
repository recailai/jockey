import { openUrl } from "@tauri-apps/plugin-opener";
import { For, Index, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { Accessor } from "solid-js";
import { marked } from "marked";
import type { AppSession, AppMessage, AppToolCall, AppSegment } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, MESSAGE_RENDER_WINDOW, fmt } from "./types";
import { assistantApi } from "../lib/tauriApi";

type MessageWindowProps = {
  activeSessionId: Accessor<string | null>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  onResetAgentContext?: () => void;
  onListMounted?: (id: string, el: HTMLElement) => void;
  onListUnmounted?: (id: string) => void;
};

const renderMd = (text: string) => marked.parse(text, { async: false }) as string;

export default function MessageWindow(props: MessageWindowProps) {
  let listEl: HTMLDivElement | undefined;
  let boundSessionId: string | null = null;

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
  const visibleMessages = createMemo<VisibleMsg[]>(() => {
    const rows = props.activeSession()?.messages ?? [];
    const sliced = rows.length <= MESSAGE_RENDER_WINDOW ? rows : rows.slice(rows.length - MESSAGE_RENDER_WINDOW);
    const deduped: VisibleMsg[] = [];
    for (const msg of sliced) {
      if (msg.roleName === "system" || msg.roleName === "event") {
        const last = deduped[deduped.length - 1];
        if (last && (last.msg.roleName === "system" || last.msg.roleName === "event") && last.msg.text === msg.text) {
          last.count++;
          last.latestAt = msg.at;
          continue;
        }
      }
      deduped.push({ msg, count: 1, latestAt: msg.at });
    }
    return deduped;
  });

  const hiddenMessageCount = (): number => {
    const count = (props.activeSession()?.messages.length ?? 0) - MESSAGE_RENDER_WINDOW;
    return count > 0 ? count : 0;
  };

  function handleContainerClick(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
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
    onCleanup(() => window.removeEventListener("click", closeCtxMenu));
  });

  return (
    <div
      ref={listEl}
      id={`msg-list-${props.activeSessionId()}`}
      role="log"
      aria-live="polite"
      aria-relevant="additions text"
      class="flex-1 overflow-auto px-4 py-4 space-y-4 bg-[#09090b] bg-[radial-gradient(ellipse_80%_80%_at_50%_-20%,rgba(120,119,198,0.08),rgba(255,255,255,0))]"
      onClick={handleContainerClick}
    >
      <Show when={hiddenMessageCount() > 0}>
        <div class="py-1 text-center text-xs text-zinc-600 opacity-40">
          {hiddenMessageCount()} older messages hidden for performance
        </div>
      </Show>
      <For each={visibleMessages()}>
        {(item) => {
          const msg = item.msg;
          if (msg.roleName === "user") return (
            <div class="flex flex-col items-end w-full mb-3 group/user">
              <div class="max-w-[85%] bg-gradient-to-br from-zinc-800 to-zinc-900 rounded-2xl rounded-tr-md px-4 py-2 text-[13px] text-zinc-100 shadow-lg border border-white/[0.08] ring-1 ring-black/20">
                <div class="whitespace-pre-wrap break-words leading-relaxed font-mono">{msg.text}</div>
              </div>
              <div class="mt-1.5 text-[10px] text-zinc-500 mr-1 opacity-0 transition-opacity duration-300 group-hover/user:opacity-100 tracking-wide">{fmt(msg.at)}</div>
            </div>
          );
          if (msg.roleName === "system" || msg.roleName === "event") return (
            <div class="flex justify-center my-3 relative">
              <div class="absolute inset-x-0 top-1/2 h-px bg-gradient-to-r from-transparent via-zinc-800/60 to-transparent -z-10"></div>
              <div class="max-w-[90%] px-4 py-1.5 bg-zinc-900/80 border border-zinc-700/50 rounded-full text-[11.5px] text-zinc-400 flex items-center gap-2.5 backdrop-blur-md shadow-sm">
                <span class="opacity-70 text-indigo-400 mt-[1px] font-serif">✧</span>
                <span class="whitespace-pre-wrap break-words tracking-wide">{msg.text}</span>
                <Show when={item.count > 1}>
                  <span class="bg-indigo-500/15 border border-indigo-500/20 text-indigo-300 font-mono rounded-full px-1.5 py-0.5 text-[9px] min-w-[20px] text-center">{item.count}</span>
                </Show>
              </div>
            </div>
          );
          return (
            <div class="flex gap-4 w-full max-w-[95%] mb-6 group/agent">
              <button
                type="button"
                onContextMenu={handleResetContextMenu}
                class="relative h-8 w-8 shrink-0 rounded-full bg-gradient-to-b from-zinc-800 to-zinc-900 border border-zinc-700/80 hover:border-indigo-400/60 hover:bg-zinc-800 transition-colors flex items-center justify-center shadow-lg ring-1 ring-black/40 mt-0.5 overflow-hidden cursor-pointer"
                title="Right-click to reset current agent CLI context"
              >
                <div class="absolute inset-0 bg-indigo-500/10"></div>
                <svg class="w-4 h-4 text-zinc-300 relative z-10 drop-shadow-sm" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" />
                </svg>
              </button>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2.5 mb-1.5 opacity-90">
                  <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.runtimeKind ?? ""] ?? "text-zinc-300"}`}>
                    {msg.roleName}
                  </span>
                  <span class="text-[10px] text-zinc-500 font-medium">{fmt(msg.at)}</span>
                  <Show when={props.activeSession()?.currentMode}>
                    <span class="rounded-md bg-indigo-500/15 border border-indigo-500/30 px-2 py-0.5 text-[9px] font-semibold text-indigo-300 uppercase tracking-wider">{props.activeSession()?.currentMode}</span>
                  </Show>
                </div>
                <Show when={msg.segments && msg.segments.length > 0} fallback={
                  <div class="md-prose" innerHTML={renderMd(msg.text)} />
                }>
                  <SegmentList segments={msg.segments!} />
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
              class="relative h-8 w-8 shrink-0 rounded-full bg-gradient-to-b from-zinc-800 to-zinc-900 border border-zinc-700/80 hover:border-indigo-400/60 hover:bg-zinc-800 transition-colors flex items-center justify-center shadow-lg ring-1 ring-black/40 mt-0.5 overflow-hidden cursor-pointer"
              title="Right-click to reset current agent CLI context"
            >
              <div class="absolute inset-0 bg-indigo-500/10"></div>
              <span class="h-1.5 w-1.5 rounded-full bg-zinc-300 animate-pulse shadow-[0_0_8px_rgba(255,255,255,0.8)] relative z-10" />
            </button>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2.5 mb-2">
                <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.runtimeKind ?? ""] ?? "text-zinc-300"}`}>
                  {props.activeSession()?.activeRole ?? "Agent"}
                </span>
                <span class="text-[10px] text-zinc-500 font-medium animate-pulse tracking-wide">
                  {(props.activeSession()?.streamSegments ?? []).length > 0
                    ? "streaming"
                    : (props.activeSession()?.agentState || "thinking...")}
                </span>
              </div>
              <Show when={(props.activeSession()?.streamSegments ?? []).length > 0} fallback={
                <>
                  <Show when={streaming().text}>
                    <div class="md-prose" innerHTML={renderMd(streaming().text)} />
                  </Show>
                  <Show when={!streaming().text && props.activeSession()?.agentState}>
                    <div class="text-[11px] text-zinc-500 italic">{props.activeSession()?.agentState}</div>
                  </Show>
                </>
              }>
                <StreamSegmentList segments={props.activeSession()?.streamSegments ?? []} />
              </Show>
              <Show when={props.activeSession()?.thoughtText}>
                <div class="mt-2 rounded-md border border-zinc-700/60 bg-zinc-900/50 px-2.5 py-2 text-[11px] text-zinc-400 leading-relaxed font-mono whitespace-pre-wrap break-words">
                  {props.activeSession()?.thoughtText}
                </div>
              </Show>
            </div>
          </div>
        )}
      </Show>
      <PermissionModal
        activeSession={props.activeSession}
        patchActiveSession={props.patchActiveSession}
      />
      <Show when={props.activeSession()?.currentPlan}>
        {(plan) => (
          <div class="rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 my-2">
            <div class="mb-1 text-xs font-semibold text-zinc-400">Plan</div>
            <ol class="list-inside list-decimal space-y-0.5 text-xs">
              <For each={plan()}>{(entry) => (
                <li class="flex items-center gap-1.5">
                  <span class={`inline-block h-1.5 w-1.5 rounded-full ${entry.status === "completed" ? "bg-emerald-400" : entry.status === "in_progress" ? "bg-amber-400 animate-pulse" : "bg-zinc-500"}`} />
                  <span class="text-zinc-300">{entry.content ?? entry.title ?? entry.description ?? "step"}</span>
                </li>
              )}</For>
            </ol>
          </div>
        )}
      </Show>
      <Show when={props.activeSession()?.submitting}>
        <div class="flex items-center gap-2 px-1 text-xs text-zinc-500 opacity-80 mt-2">
          <span class="h-2 w-2 rounded-full bg-white/60 animate-pulse" />
          <span>{props.activeSession()?.agentState || "Agent is thinking..."}</span>
        </div>
      </Show>
      <Show when={(props.activeSession()?.queuedMessages ?? []).length > 0}>
        <div class="mt-3 rounded-lg border border-zinc-700/40 bg-zinc-900/30 backdrop-blur-sm overflow-hidden">
          <div class="flex items-center gap-2 px-3 py-1.5 border-b border-zinc-700/30">
            <span class="h-1.5 w-1.5 rounded-full bg-indigo-400 animate-pulse" />
            <span class="text-[10px] text-zinc-400 font-medium uppercase tracking-wider">Queued</span>
            <span class="ml-auto text-[9px] text-zinc-500 font-mono bg-zinc-800/60 px-1.5 py-0.5 rounded-md">{props.activeSession()!.queuedMessages.length}</span>
          </div>
          <div class="px-2 py-1.5 space-y-1">
            <For each={props.activeSession()!.queuedMessages}>{(text, i) => (
              <div class="flex items-start gap-2 px-2 py-1 rounded-md hover:bg-zinc-800/30 transition-colors">
                <span class="text-[9px] text-zinc-500 font-mono mt-0.5 shrink-0 w-4 text-right">{i() + 1}</span>
                <span class="text-[11px] text-zinc-300 font-mono break-all leading-relaxed">{text}</span>
              </div>
            )}</For>
          </div>
        </div>
      </Show>
      <Show when={ctxMenu()}>
        {(pos) => (
          <div
            class="fixed z-[200] min-w-[140px] overflow-hidden rounded-lg border border-white/[0.08] bg-zinc-900/95 shadow-xl shadow-black/60 backdrop-blur-md py-1"
            style={`left:${pos().x}px;top:${pos().y}px`}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-zinc-300 hover:bg-white/[0.06] hover:text-white transition-colors"
              onClick={() => { closeCtxMenu(); props.onResetAgentContext?.(); }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 text-zinc-500">
                <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>
              </svg>
              Reset context
            </button>
          </div>
        )}
      </Show>
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

function SegmentList(props: { segments: AppSegment[] }) {
  const groups = createMemo(() => collectToolGroups(props.segments));
  return (
    <For each={groups()}>{(g) => (
      g.kind === "text"
        ? <div class="md-prose" innerHTML={renderMd(g.text)} />
        : <ToolCallGroup tools={g.tools} streaming={false} />
    )}</For>
  );
}

function StreamSegmentList(props: { segments: AppSegment[] }) {
  const groups = createMemo(() => collectToolGroups(props.segments));
  return (
    <Index each={groups()}>{(g) => (
      g().kind === "text"
        ? <div class="md-prose" innerHTML={renderMd((g() as { kind: "text"; text: string }).text)} />
        : <ToolCallGroup tools={(g() as { kind: "tools"; tools: AppToolCall[] }).tools} streaming={true} />
    )}</Index>
  );
}

function tcStatusDot(status: string): string {
  if (status === "success" || status === "completed") return "bg-emerald-400 shadow-emerald-400/40";
  if (status === "failure" || status === "error") return "bg-rose-400 shadow-rose-400/40";
  return "bg-amber-400 animate-pulse shadow-amber-400/40";
}

function ToolCallGroup(props: { tools: AppToolCall[]; streaming: boolean }) {
  const [expanded, setExpanded] = createSignal(false);

  const count = () => props.tools.length;
  const lastTool = () => props.tools[props.tools.length - 1];
  const statusCounts = createMemo(() => {
    let success = 0, error = 0;
    for (const t of props.tools) {
      if (t.status === "success" || t.status === "completed") success++;
      else if (t.status === "failure" || t.status === "error") error++;
    }
    return { success, error, pending: props.tools.length - success - error };
  });

  return (
    <div class="rounded-xl border border-white/[0.05] bg-zinc-900/40 backdrop-blur-md overflow-hidden shadow-sm my-1.5">
      <button
        class="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-[11.5px] text-zinc-300 select-none hover:bg-zinc-900/60 transition-colors"
        onClick={() => setExpanded(v => !v)}
      >
        <span class={`h-2 w-2 shrink-0 rounded-full shadow-[0_0_8px_rgba(0,0,0,0.5)] ${tcStatusDot(lastTool()?.status ?? "pending")}`} />
        <span class="text-zinc-200 font-mono tracking-tight font-medium">{lastTool()?.title || "tool calls"}</span>
        <span class="flex items-center gap-1.5 ml-auto">
          <Show when={statusCounts().success > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-emerald-400">
              <span class="h-1.5 w-1.5 rounded-full bg-emerald-400 inline-block" />
              {statusCounts().success}
            </span>
          </Show>
          <Show when={statusCounts().error > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-rose-400">
              <span class="h-1.5 w-1.5 rounded-full bg-rose-400 inline-block" />
              {statusCounts().error}
            </span>
          </Show>
          <Show when={statusCounts().pending > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-amber-400">
              <span class="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse inline-block" />
              {statusCounts().pending}
            </span>
          </Show>
          <span class="text-[9px] text-zinc-500 font-bold tracking-widest uppercase bg-zinc-800/50 px-1.5 py-0.5 rounded-md ml-1">{count()} calls</span>
          <svg class={`w-3 h-3 text-zinc-500 transition-transform duration-150 ${expanded() ? "rotate-180" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
        </span>
      </button>
      <Show when={expanded()}>
        <div class="border-t border-white/[0.05] px-2 py-1.5 space-y-1">
          <For each={props.tools}>{(tc) => (
            <ToolCallItem tc={tc} />
          )}</For>
        </div>
      </Show>
    </div>
  );
}

function ToolCallItem(props: { tc: AppToolCall }) {
  const tc = () => props.tc;
  return (
    <details class="group/tc rounded-lg border border-white/[0.04] bg-zinc-900/30 overflow-hidden transition-all duration-200 hover:bg-zinc-900/50 hover:border-white/[0.08]">
      <summary class="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-[11px] text-zinc-300 select-none">
        <span class={`h-1.5 w-1.5 shrink-0 rounded-full shadow-[0_0_6px_rgba(0,0,0,0.4)] ${tcStatusDot(tc().status)}`} />
        <span class="text-zinc-200 font-mono tracking-tight font-medium group-hover/tc:text-white transition-colors truncate">{tc().title || tc().toolCallId}</span>
        <span class="ml-auto text-[9px] text-zinc-500 uppercase tracking-widest font-bold bg-zinc-800/50 px-1.5 py-0.5 rounded-md shrink-0">{tc().kind}</span>
      </summary>
      <Show when={tc().contentJson}>
        <pre class="whitespace-pre-wrap break-words border-t border-white/[0.05] px-3 py-2.5 text-[11px] font-mono text-zinc-400 bg-black/20">{tc().contentJson}</pre>
      </Show>
      <Show when={tc().locations && tc().locations!.length > 0}>
        <div class="border-t border-white/[0.05] px-3 py-2 text-[10.5px] text-zinc-400 bg-black/10">
          <div class="mb-0.5 uppercase tracking-wider text-[9px] text-zinc-500">Files</div>
          <For each={tc().locations}>{(loc) => (
            <div class="font-mono break-all">{loc.path}{loc.line ? `:${loc.line}` : ""}</div>
          )}</For>
        </div>
      </Show>
      <Show when={tc().rawInputJson}>
        <pre class="whitespace-pre-wrap break-words border-t border-white/[0.05] px-3 py-2.5 text-[10.5px] font-mono text-zinc-500 bg-black/10">{tc().rawInputJson}</pre>
      </Show>
      <Show when={tc().rawOutputJson}>
        <pre class="whitespace-pre-wrap break-words border-t border-white/[0.05] px-3 py-2.5 text-[10.5px] font-mono text-zinc-500 bg-black/10">{tc().rawOutputJson}</pre>
      </Show>
    </details>
  );
}

type PermissionModalProps = {
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
};

function PermissionModal(props: PermissionModalProps) {
  return (
    <Show when={props.activeSession()?.pendingPermission}>
      {(perm) => (
        <div class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 my-2">
          <div class="mb-1 text-xs font-semibold text-amber-300">{perm().title}</div>
          <Show when={perm().description}><p class="mb-2 text-xs text-zinc-400">{perm().description}</p></Show>
          <div class="flex gap-2">
            <For each={perm().options}>{(opt) => (
              <button
                class={`min-h-8 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
                onClick={() => {
                  void assistantApi.respondPermission(perm().requestId, opt.optionId, false);
                  props.patchActiveSession({ pendingPermission: null });
                }}
              >
                {opt.title ?? opt.optionId}
              </button>
            )}</For>
            <button
              class={`min-h-8 rounded border border-rose-500/40 bg-rose-500/10 px-3 py-1 text-xs text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
              onClick={() => {
                void assistantApi.respondPermission(perm().requestId, "", true);
                props.patchActiveSession({ pendingPermission: null });
              }}
            >
              Deny
            </button>
          </div>
        </div>
      )}
    </Show>
  );
}
