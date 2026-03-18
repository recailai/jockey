import { invoke } from "@tauri-apps/api/core";
import { For, Show, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { AppSession, AssistantRuntime } from "./types";
import { INTERACTIVE_MOTION } from "./types";

type SessionTabsProps = {
  sessions: AppSession[];
  activeSessionId: Accessor<string | null>;
  setActiveSessionId: Setter<string | null>;
  activeSession: Accessor<AppSession | null>;
  assistants: Accessor<AssistantRuntime[]>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  activeBackendRole: () => string;
  onNewSession: () => void;
  onCloseSession: (id: string) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  onRefresh: () => void;
  onToggleDrawer: () => void;
};

export default function SessionTabs(props: SessionTabsProps) {
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");

  return (
    <div class="flex h-9 shrink-0 items-center border-b border-white/[0.06] bg-[#0d0d10] gap-1" style="padding-left: max(8px, env(titlebar-area-x, 78px)); padding-right: 8px;">
      <For each={props.sessions}>
        {(s) => (
          <div
            onClick={() => { if (renamingSessionId() !== s.id) props.setActiveSessionId(s.id); }}
            onDblClick={() => {
              setRenamingSessionId(s.id);
              setRenameValue(s.title);
            }}
            class={`group relative flex items-center gap-1.5 rounded-md px-3 py-1 text-xs transition-colors cursor-default select-none ${
              s.id === props.activeSessionId()
                ? "bg-white/[0.08] text-white"
                : "text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.04]"
            }`}
          >
            <Show when={s.status === "running"}>
              <span class="h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse" />
            </Show>
            <Show when={s.status === "done" && s.id !== props.activeSessionId()}>
              <span class="h-1.5 w-1.5 rounded-full bg-blue-500" />
            </Show>
            <Show when={renamingSessionId() === s.id} fallback={
              <span class="max-w-[120px] truncate">{s.title}</span>
            }>
              <input
                class="max-w-[120px] bg-transparent outline-none border-b border-white/30 text-xs text-white"
                value={renameValue()}
                onInput={(e) => setRenameValue(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    const val = renameValue().trim();
                    if (val) {
                      props.updateSession(s.id, { title: val });
                      void invoke("update_app_session", { id: s.id, update: { title: val } }).catch(() => {});
                    }
                    setRenamingSessionId(null);
                  } else if (e.key === "Escape") {
                    setRenamingSessionId(null);
                  }
                }}
                onBlur={() => {
                  const val = renameValue().trim();
                  if (val) {
                    props.updateSession(s.id, { title: val });
                    void invoke("update_app_session", { id: s.id, update: { title: val } }).catch(() => {});
                  }
                  setRenamingSessionId(null);
                }}
                ref={(el) => queueMicrotask(() => el?.select())}
                onClick={(e) => e.stopPropagation()}
              />
            </Show>
            <Show when={props.sessions.length > 1}>
              <button
                onClick={(e) => { e.stopPropagation(); props.onCloseSession(s.id); }}
                class="ml-0.5 opacity-0 group-hover:opacity-60 hover:!opacity-100 text-zinc-400 leading-none"
              >x</button>
            </Show>
          </div>
        )}
      </For>
      <button
        onClick={() => props.onNewSession()}
        class="flex h-6 w-6 items-center justify-center rounded text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] text-sm"
      >+</button>
      <div class="flex-1" />
      <For each={props.assistants()}>
        {(a) => (
          <button
            onClick={() => a.available && props.patchActiveSession({ selectedAssistant: a.key })}
            class={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] transition-colors ${
              props.activeSession()?.selectedAssistant === a.key
                ? "bg-white/[0.12] text-white"
                : "text-zinc-600 hover:text-zinc-400"
            } ${!a.available ? "opacity-30 pointer-events-none" : ""}`}
          >
            <span class={`h-1.5 w-1.5 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
            {a.label}
          </button>
        )}
      </For>
      <Show when={(props.activeSession()?.agentModes ?? []).length > 0}>
        <div class="flex gap-1 ml-1">
          <For each={props.activeSession()?.agentModes ?? []}>
            {(m) => (
              <button
                class={`min-h-6 rounded border px-2 py-0.5 text-[10px] ${INTERACTIVE_MOTION} ${props.activeSession()?.currentMode === m.id ? "border-white/25 bg-white/10 text-white" : "border-white/[0.06] text-zinc-600 hover:text-zinc-300"}`}
                onClick={() => {
                  const assistant = props.activeSession()?.selectedAssistant ?? null;
                  const role = props.activeBackendRole();
                  if (assistant) void invoke("set_acp_mode", { runtimeKind: assistant, roleName: role, modeId: m.id });
                }}
              >
                {m.title ?? m.id}
              </button>
            )}
          </For>
        </div>
      </Show>
      <button
        onClick={() => props.onRefresh()}
        class={`flex h-7 w-7 items-center justify-center rounded-lg text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] ${INTERACTIVE_MOTION}`}
        title="Refresh"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
      </button>
      <button
        onClick={() => props.onToggleDrawer()}
        class={`flex h-7 w-7 items-center justify-center rounded-lg text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] ${INTERACTIVE_MOTION}`}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <circle cx="12" cy="12" r="3"/><path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83"/>
        </svg>
      </button>
    </div>
  );
}
