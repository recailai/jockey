import { invoke } from "@tauri-apps/api/core";
import { For, Show, createEffect, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, AppMessage } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, MESSAGE_RENDER_WINDOW, fmt } from "./types";

type MessageWindowProps = {
  activeSessionId: Accessor<string | null>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  onListMounted?: (id: string, el: HTMLElement) => void;
  onListUnmounted?: (id: string) => void;
};

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

  const visibleMessages = (): (AppMessage & { count?: number })[] => {
    const rows = props.activeSession()?.messages ?? [];
    const sliced = rows.length <= MESSAGE_RENDER_WINDOW ? rows : rows.slice(rows.length - MESSAGE_RENDER_WINDOW);
    const deduped: (AppMessage & { count?: number })[] = [];
    for (const msg of sliced) {
      if (msg.role === "system" || msg.role === "event") {
        const last = deduped[deduped.length - 1];
        if (last && (last.role === "system" || last.role === "event") && last.text === msg.text) {
          last.count = (last.count ?? 1) + 1;
          last.at = msg.at;
          continue;
        }
      }
      deduped.push({ ...msg, count: 1 });
    }
    return deduped;
  };

  const hiddenMessageCount = (): number => {
    const count = (props.activeSession()?.messages.length ?? 0) - MESSAGE_RENDER_WINDOW;
    return count > 0 ? count : 0;
  };

  return (
    <div
      ref={listEl}
      id={`msg-list-${props.activeSessionId()}`}
      role="log"
      aria-live="polite"
      aria-relevant="additions text"
      class="flex-1 overflow-auto px-4 py-4 space-y-4 bg-[#09090b] bg-[radial-gradient(ellipse_80%_80%_at_50%_-20%,rgba(120,119,198,0.08),rgba(255,255,255,0))]"
    >
      <Show when={hiddenMessageCount() > 0}>
        <div class="py-1 text-center text-xs text-zinc-600 opacity-40">
          {hiddenMessageCount()} older messages hidden for performance
        </div>
      </Show>
      <For each={visibleMessages()}>
        {(msg) => {
          if (msg.role === "user") return (
            <div class="flex flex-col items-end w-full mb-3 group/user">
              <div class="max-w-[85%] bg-gradient-to-br from-zinc-800 to-zinc-900 rounded-2xl rounded-tr-md px-4 py-2 text-[13px] text-zinc-100 shadow-lg border border-white/[0.08] ring-1 ring-black/20">
                <div class="whitespace-pre-wrap break-words leading-relaxed font-mono">{msg.text}</div>
              </div>
              <div class="mt-1.5 text-[10px] text-zinc-500 mr-1 opacity-0 transition-opacity duration-300 group-hover/user:opacity-100 tracking-wide">{fmt(msg.at)}</div>
            </div>
          );
          if (msg.role === "system" || msg.role === "event") return (
            <div class="flex justify-center my-3 relative">
              <div class="absolute inset-x-0 top-1/2 h-px bg-gradient-to-r from-transparent via-zinc-800/60 to-transparent -z-10"></div>
              <div class="max-w-[90%] px-4 py-1.5 bg-zinc-900/80 border border-zinc-700/50 rounded-full text-[11.5px] text-zinc-400 flex items-center gap-2.5 backdrop-blur-md shadow-sm">
                <span class="opacity-70 text-indigo-400 mt-[1px] font-serif">✧</span>
                <span class="truncate tracking-wide">{msg.text}</span>
                <Show when={(msg.count ?? 1) > 1}>
                  <span class="bg-indigo-500/15 border border-indigo-500/20 text-indigo-300 font-mono rounded-full px-1.5 py-0.5 text-[9px] min-w-[20px] text-center">{msg.count}</span>
                </Show>
              </div>
            </div>
          );
          return (
            <div class="flex gap-4 w-full max-w-[95%] mb-6 group/agent">
              <div class="relative h-8 w-8 shrink-0 rounded-full bg-gradient-to-b from-zinc-800 to-zinc-900 border border-zinc-700/80 flex items-center justify-center shadow-lg ring-1 ring-black/40 mt-0.5 overflow-hidden">
                <div class="absolute inset-0 bg-indigo-500/10"></div>
                <svg class="w-4 h-4 text-zinc-300 relative z-10 drop-shadow-sm" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" />
                </svg>
              </div>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2.5 mb-1.5 opacity-90">
                  <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-300"}`}>
                    {msg.roleLabel ?? "Agent"}
                  </span>
                  <span class="text-[10px] text-zinc-500 font-medium">{fmt(msg.at)}</span>
                  <Show when={props.activeSession()?.currentMode}>
                    <span class="rounded-md bg-indigo-500/15 border border-indigo-500/30 px-2 py-0.5 text-[9px] font-semibold text-indigo-300 uppercase tracking-wider">{props.activeSession()?.currentMode}</span>
                  </Show>
                </div>
                <Show when={msg.toolCalls && msg.toolCalls.length > 0}>
                  <div class="space-y-2 mb-3">
                    <For each={msg.toolCalls}>{(tc) => (
                      <details class="group/tc rounded-xl border border-white/[0.05] bg-zinc-900/40 backdrop-blur-md overflow-hidden shadow-sm transition-all duration-200 hover:bg-zinc-900/60 hover:border-white/[0.1]">
                        <summary class="flex cursor-pointer items-center gap-2.5 px-3 py-2.5 text-[11.5px] text-zinc-300 select-none">
                          <span class={`h-2 w-2 shrink-0 rounded-full shadow-[0_0_8px_rgba(0,0,0,0.5)] ${tc.status === "success" || tc.status === "completed" ? "bg-emerald-400 shadow-emerald-400/40" : tc.status === "failure" || tc.status === "error" ? "bg-rose-400 shadow-rose-400/40" : "bg-amber-400 animate-pulse shadow-amber-400/40"}`} />
                          <span class="text-zinc-200 font-mono tracking-tight font-medium group-hover/tc:text-white transition-colors">{tc.title || tc.toolCallId}</span>
                          <span class="ml-auto text-[9px] text-zinc-500 uppercase tracking-widest font-bold bg-zinc-800/50 px-1.5 py-0.5 rounded-md">{tc.kind}</span>
                        </summary>
                        <Show when={tc.contentJson}>
                          <pre class="whitespace-pre-wrap break-words border-t border-white/[0.05] px-3.5 py-3 text-[11.5px] font-mono text-zinc-400 bg-black/20">{tc.contentJson}</pre>
                        </Show>
                      </details>
                    )}</For>
                  </div>
                </Show>
                <div class="whitespace-pre-wrap break-words text-[13.5px] text-zinc-200 leading-[1.7] font-mono">{msg.text}</div>
              </div>
            </div>
          );
        }}
      </For>
      <Show when={props.activeSession()?.streamingMessage}>
        {(streaming) => (
          <div class="flex gap-4 w-full max-w-[95%] mb-6">
            <div class="relative h-8 w-8 shrink-0 rounded-full bg-gradient-to-b from-zinc-800 to-zinc-900 border border-zinc-700/80 flex items-center justify-center shadow-lg ring-1 ring-black/40 mt-0.5 overflow-hidden">
              <div class="absolute inset-0 bg-indigo-500/10"></div>
              <span class="h-1.5 w-1.5 rounded-full bg-zinc-300 animate-pulse shadow-[0_0_8px_rgba(255,255,255,0.8)] relative z-10" />
            </div>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2.5 mb-2">
                <span class={`text-[12px] font-bold tracking-wider uppercase ${RUNTIME_COLOR[props.activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-300"}`}>
                  {props.activeSession()?.activeRole ?? "Agent"}
                </span>
                <span class="text-[10px] text-zinc-500 font-medium animate-pulse tracking-wide">thinking...</span>
              </div>
              <div class="whitespace-pre-wrap break-words text-[13.5px] text-zinc-200 leading-[1.7] font-mono">{streaming().text}</div>
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
                  <span class="text-zinc-300">{entry.title ?? entry.description ?? "step"}</span>
                </li>
              )}</For>
            </ol>
          </div>
        )}
      </Show>
      <Show when={(props.activeSession()?.toolCalls.size ?? 0) > 0}>
        <div class="space-y-1 my-2">
          <For each={[...(props.activeSession()?.toolCalls.values() ?? [])]}>{(tc) => (
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
      <Show when={props.activeSession()?.submitting}>
        <div class="flex items-center gap-2 px-1 text-xs text-zinc-500 opacity-80 mt-2">
          <span class="h-2 w-2 rounded-full bg-white/60 animate-pulse" />
          <span>{props.activeSession()?.agentState || "Agent is thinking..."}</span>
        </div>
      </Show>
    </div>
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
                  void invoke("respond_permission", { requestId: perm().requestId, optionId: opt.optionId, cancelled: false });
                  props.patchActiveSession({ pendingPermission: null });
                }}
              >
                {opt.title ?? opt.optionId}
              </button>
            )}</For>
            <button
              class={`min-h-8 rounded border border-rose-500/40 bg-rose-500/10 px-3 py-1 text-xs text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
              onClick={() => {
                void invoke("respond_permission", { requestId: perm().requestId, optionId: "", cancelled: true });
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
