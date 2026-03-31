import { For, Show, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { AppSession } from "./types";
import { INTERACTIVE_MOTION } from "./types";
import { appSessionApi, assistantApi } from "../lib/tauriApi";

type SessionTabsProps = {
  sessions: AppSession[];
  activeSessionId: Accessor<string | null>;
  setActiveSessionId: Setter<string | null>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  activeBackendRole: () => string;
  onNewSession: () => void;
  onCloseSession: (id: string) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  onRefresh: () => void;
  onToggleDrawer: () => void;
  onToggleManagement?: () => void;
};

export default function SessionTabs(props: SessionTabsProps) {
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");
  const isValidSessionName = (sessionId: string, raw: string): string | null => {
    const val = raw.trim();
    if (!val) return null;
    if (/\s/.test(val)) return null;
    if (props.sessions.some((s) => s.id !== sessionId && s.title.toLowerCase() === val.toLowerCase())) return null;
    return val;
  };
  const commitRename = async (sessionId: string) => {
    const val = isValidSessionName(sessionId, renameValue());
    if (!val) {
      setRenamingSessionId(null);
      return;
    }
    try {
      await appSessionApi.update(sessionId, { title: val });
      props.updateSession(sessionId, { title: val });
    } catch {}
    setRenamingSessionId(null);
  };

  return (
    <div data-tauri-drag-region class="flex h-[38px] shrink-0 items-center gap-1.5 relative z-10" style="padding-left: max(12px, env(titlebar-area-x, 78px)); padding-right: 10px;">
      <For each={props.sessions}>
        {(s) => (
          <div
            onClick={() => { if (renamingSessionId() !== s.id) props.setActiveSessionId(s.id); }}
            onDblClick={() => {
              setRenamingSessionId(s.id);
              setRenameValue(s.title);
            }}
            class={`group relative flex items-center gap-1.5 rounded-full px-3 py-1 text-[10.5px] font-bold tracking-wide transition-all duration-200 cursor-default select-none border shadow-sm ${
              s.id === props.activeSessionId()
                ? "text-primary border-theme-strong ring-1 ring-black/20"
                : "border-transparent theme-muted hover:text-primary"
            }`}
            style={s.id === props.activeSessionId() ? { background: "var(--ui-surface-muted)" } : { background: "var(--ui-panel)" }}
          >
            <Show when={s.status === "running"}>
              <span class="h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse shadow-[0_0_6px_rgba(96,165,250,0.6)]" />
            </Show>
            <Show when={s.status === "error"}>
              <span class="h-1.5 w-1.5 rounded-full bg-rose-400 shadow-[0_0_6px_rgba(251,113,133,0.6)]" />
            </Show>
            <Show when={s.status === "done" && s.id !== props.activeSessionId()}>
              <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" />
            </Show>
            <Show when={renamingSessionId() === s.id} fallback={
              <span class="max-w-[120px] truncate">{s.title}</span>
            }>
              <input
                class="max-w-[120px] bg-transparent outline-none border-b border-[var(--ui-border-strong)] text-[10.5px] theme-text font-bold tracking-wide"
                value={renameValue()}
                onInput={(e) => setRenameValue(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    void commitRename(s.id);
                  } else if (e.key === "Escape") {
                    setRenamingSessionId(null);
                  }
                }}
                onBlur={() => {
                  void commitRename(s.id);
                }}
                ref={(el) => queueMicrotask(() => el?.select())}
                onClick={(e) => e.stopPropagation()}
              />
            </Show>
            <Show when={props.sessions.length > 1}>
              <button
                onClick={(e) => { e.stopPropagation(); props.onCloseSession(s.id); }}
                class="ml-1 opacity-0 group-hover:opacity-70 hover:!opacity-100 theme-muted hover:text-rose-400 transition-colors leading-none"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </Show>
          </div>
        )}
      </For>
      <button
        onClick={() => props.onNewSession()}
        class="flex h-6 w-6 items-center justify-center rounded-md theme-muted hover:text-primary hover:bg-[var(--ui-accent-soft)] transition-all motion-safe:hover:scale-105 mx-0.5"
        title="New Session"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
      </button>
      <div class="flex-1" />
      <Show when={(props.activeSession()?.agentModes ?? []).length > 0}>
        <div class="flex gap-1.5 ml-1.5">
          <For each={props.activeSession()?.agentModes ?? []}>
            {(m) => (
              <button
                class={`min-h-6 rounded-md border px-2 py-px text-[9px] font-bold tracking-wider uppercase transition-all duration-200 ${INTERACTIVE_MOTION} ${props.activeSession()?.currentMode === m.id ? "border-indigo-500/40 bg-indigo-500/20 text-indigo-300 shadow-[0_0_8px_rgba(99,102,241,0.15)] ring-1 ring-indigo-500/20" : "border-[var(--ui-border)] bg-[var(--ui-surface-muted)] theme-muted hover:text-primary hover:bg-[var(--ui-surface-muted)] hover:border-[var(--ui-border-strong)]"}`}
                onClick={() => {
                  const assistant = props.activeSession()?.runtimeKind ?? null;
                  const role = props.activeBackendRole();
                  if (assistant) void assistantApi.setMode(assistant, role, m.id, props.activeSessionId() ?? "");
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
        class={`flex h-6 w-6 items-center justify-center rounded-md theme-muted hover:text-primary hover:bg-[var(--ui-accent-soft)] transition-all motion-safe:hover:scale-105 ml-1 ${INTERACTIVE_MOTION}`}
        title="Refresh"
      >
        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
      </button>
      <button
        onClick={() => props.onToggleDrawer()}
        class={`flex h-6 w-6 items-center justify-center rounded-md theme-muted hover:text-primary hover:bg-[var(--ui-accent-soft)] transition-all motion-safe:hover:scale-105 ${INTERACTIVE_MOTION}`}
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3"/><path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83"/>
        </svg>
      </button>
    </div>
  );
}
