import { For, Show, createEffect } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { AppSession, AppMentionItem } from "./types";
import { INTERACTIVE_MOTION, DEFAULT_ROLE_ALIAS } from "./types";

type ChatInputProps = {
  input: Accessor<string>;
  setInput: Setter<string>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  isCustomRole: () => boolean;
  onSubmit: (e: SubmitEvent) => void;
  onInputEvent: (el: HTMLInputElement) => void;
  onInputKeyDown: (e: KeyboardEvent) => void;
  refreshInputCompletions: (value: string, caret: number) => void;
  mentionOpen: Accessor<boolean>;
  mentionItems: Accessor<AppMentionItem[]>;
  mentionActiveIndex: Accessor<number>;
  slashOpen: Accessor<boolean>;
  slashItems: Accessor<AppMentionItem[]>;
  slashActiveIndex: Accessor<number>;
  applyMentionCandidate: (item: AppMentionItem) => void;
  applySlashCandidate: (item: AppMentionItem) => void;
  closeMentionMenu: () => void;
  closeSlashMenu: () => void;
  inputElRef: (el: HTMLInputElement) => void;
  mentionCloseTimerRef: { current: number | null };
  mentionDebounceTimerRef: { current: number | null };
};

function mentionKindColor(kind: string): string {
  if (kind === "role") return "bg-blue-500/20 text-blue-200";
  if (kind === "dir") return "bg-emerald-500/20 text-emerald-200";
  if (kind === "file") return "bg-amber-500/20 text-amber-200";
  if (kind === "command") return "bg-indigo-500/20 text-indigo-200";
  if (kind === "skill") return "bg-violet-500/20 text-violet-200";
  return "bg-zinc-500/20 text-zinc-300";
}

export default function ChatInput(props: ChatInputProps) {
  createEffect(() => {
    const idx = props.slashActiveIndex();
    const container = slashListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  createEffect(() => {
    const idx = props.mentionActiveIndex();
    const container = mentionListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  let slashListEl: HTMLDivElement | undefined;
  let mentionListEl: HTMLDivElement | undefined;

  return (
    <div class="shrink-0 px-4 py-3 bg-[#09090b] border-t border-white/[0.04]">
      <form onSubmit={props.onSubmit} class="relative">
        <div class="flex items-center rounded-full border border-white/[0.1] bg-white/[0.04] px-4 gap-2 focus-within:border-white/[0.18] motion-safe:transition-colors">
          <button
            type="button"
            onClick={() => props.patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS })}
            class={`shrink-0 py-2.5 text-xs font-mono ${props.isCustomRole() ? "text-blue-300 hover:text-blue-200" : "text-zinc-500"}`}
            title={props.isCustomRole() ? "Click to return to UnionAI" : "UnionAI mode"}
          >
            {props.activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS} &gt;
          </button>
          <input
            ref={(el) => { props.inputElRef(el); }}
            value={props.input()}
            onInput={(e) => props.onInputEvent(e.currentTarget)}
            onClick={(e) => props.refreshInputCompletions(e.currentTarget.value, e.currentTarget.selectionStart ?? e.currentTarget.value.length)}
            onKeyDown={props.onInputKeyDown}
            onBlur={() => {
              if (props.mentionCloseTimerRef.current !== null) window.clearTimeout(props.mentionCloseTimerRef.current);
              props.mentionCloseTimerRef.current = window.setTimeout(() => {
                props.closeMentionMenu();
                props.closeSlashMenu();
              }, 120);
            }}
            onFocus={(e) => {
              if (props.mentionCloseTimerRef.current !== null) window.clearTimeout(props.mentionCloseTimerRef.current);
              if (props.mentionDebounceTimerRef.current !== null) {
                window.clearTimeout(props.mentionDebounceTimerRef.current);
                props.mentionDebounceTimerRef.current = null;
              }
              props.refreshInputCompletions(props.input(), e.currentTarget.selectionStart ?? props.input().length);
            }}
            placeholder={props.isCustomRole() ? `Chat with ${props.activeSession()?.activeRole}... (type / for agent commands)` : "Natural language / commands / @role @file:path"}
            class="flex-1 bg-transparent py-2.5 text-sm outline-none min-w-0 text-zinc-200 placeholder:text-zinc-600"
          />
          <button
            type="submit"
            class={`shrink-0 flex h-7 w-7 items-center justify-center rounded-full motion-safe:transition-all motion-safe:duration-150 ${props.input().trim() ? "bg-white text-zinc-950" : "bg-white/[0.08] text-white/20"} ${INTERACTIVE_MOTION}`}
            title={props.activeSession()?.submitting ? "Queue" : "Send"}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>
          </button>
        </div>
        <Show when={props.slashOpen() && props.slashItems().length > 0}>
          <div ref={(el) => { slashListEl = el; }} class="absolute bottom-14 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-xl">
            <For each={props.slashItems()}>
              {(item, i) => (
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    props.applySlashCandidate(item);
                  }}
                  class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === props.slashActiveIndex() ? "bg-white/[0.08] text-white" : "text-zinc-400 hover:bg-white/[0.04]"}`}
                >
                  <span class="rounded bg-indigo-500/20 px-1 text-[10px] uppercase tracking-wide text-indigo-200">cmd</span>
                  <span class="truncate font-mono text-xs">{item.value}</span>
                  <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={props.mentionOpen() && props.mentionItems().length > 0}>
          <div ref={(el) => { mentionListEl = el; }} class="absolute bottom-14 left-0 right-0 z-30 max-h-56 overflow-auto rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-xl">
            <For each={props.mentionItems()}>
              {(item, i) => (
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    props.applyMentionCandidate(item);
                  }}
                  class={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${i() === props.mentionActiveIndex() ? "bg-white/[0.08] text-white" : "text-zinc-400 hover:bg-white/[0.04]"}`}
                >
                  <span class={`rounded px-1 text-[10px] uppercase tracking-wide ${mentionKindColor(item.kind)}`}>
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
  );
}
