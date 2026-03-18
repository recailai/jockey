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
  let listEl: HTMLElement | undefined;
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

  const visibleMessages = (): AppMessage[] => {
    const rows = props.activeSession()?.messages ?? [];
    if (rows.length <= MESSAGE_RENDER_WINDOW) return rows;
    return rows.slice(rows.length - MESSAGE_RENDER_WINDOW);
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
                <span class={`text-[10px] font-semibold ${RUNTIME_COLOR[props.activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-400"}`}>
                  [{msg.roleLabel ?? "assistant"}]
                </span>
                <span class="text-[10px] text-zinc-600">{fmt(msg.at)}</span>
                <Show when={props.activeSession()?.currentMode}>
                  <span class="rounded bg-indigo-500/20 px-1 text-[9px] text-indigo-200">{props.activeSession()?.currentMode}</span>
                </Show>
              </div>
              <Show when={msg.toolCalls && msg.toolCalls.length > 0}>
                <div class="space-y-0.5 mb-1">
                  <For each={msg.toolCalls}>{(tc) => (
                    <details class="rounded border border-zinc-800 bg-zinc-900/60">
                      <summary class="flex cursor-pointer items-center gap-2 px-2 py-1 text-[10px]">
                        <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${tc.status === "success" || tc.status === "completed" ? "bg-emerald-400" : tc.status === "failure" || tc.status === "error" ? "bg-rose-400" : "bg-zinc-500"}`} />
                        <span class="text-zinc-400">{tc.title || tc.toolCallId}</span>
                        <span class="ml-auto text-zinc-600">{tc.kind}</span>
                      </summary>
                      <Show when={tc.contentJson}>
                        <pre class="whitespace-pre-wrap break-words border-t border-zinc-800 px-2 py-1 text-[10px] text-zinc-500">{tc.contentJson}</pre>
                      </Show>
                    </details>
                  )}</For>
                </div>
              </Show>
              <pre class="whitespace-pre-wrap break-words text-zinc-300 leading-relaxed pl-0">{msg.text}</pre>
            </div>
          );
        }}
      </For>
      <Show when={props.activeSession()?.streamingMessage}>
        {(streaming) => (
          <div class="mb-3">
            <div class="flex items-center gap-2 mb-0.5">
              <span class={`text-[10px] font-semibold ${RUNTIME_COLOR[props.activeSession()?.selectedAssistant ?? ""] ?? "text-zinc-400"}`}>
                [{props.activeSession()?.activeRole ?? "assistant"}]
              </span>
              <span class="h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse" />
            </div>
            <pre class="whitespace-pre-wrap break-words text-zinc-300 leading-relaxed">{streaming().text}</pre>
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
