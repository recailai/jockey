import { For, Show, createMemo, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import { Clock3, LoaderCircle, Search, Settings, SquarePen, X } from "lucide-solid";
import type { AppSession } from "./types";
import { INTERACTIVE_MOTION } from "./types";
import { appSessionApi } from "../lib/tauriApi";
import { IconButton, RowButton } from "./ui";

type SessionSidebarProps = {
  sessions: AppSession[];
  activeSessionId: Accessor<string | null>;
  widthPx: Accessor<number>;
  setActiveSessionId: Setter<string | null>;
  onNewSession: () => void;
  onCloseSession: (id: string) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  onOpenAutomations: () => void;
  onOpenSettings: () => void;
};

function statusClass(session: AppSession): string {
  if (session.status === "error") return "ui-status-danger";
  return "ui-status-muted";
}

function isRunning(session: AppSession): boolean {
  return session.submitting || session.status === "running";
}

function statusTitle(session: AppSession): string {
  if (isRunning(session)) return "Running";
  if (session.status === "error") return "Error";
  return "Idle";
}

export default function SessionSidebar(props: SessionSidebarProps) {
  const [query, setQuery] = createSignal("");
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");

  const filteredSessions = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return props.sessions;
    return props.sessions.filter((s) =>
      s.title.toLowerCase().includes(q) ||
      (s.cwd ?? "").toLowerCase().includes(q) ||
      s.activeRole.toLowerCase().includes(q)
    );
  });

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
    <aside
      data-tauri-drag-region
      class="session-rail flex h-full shrink-0 flex-col"
      style={{ width: `${props.widthPx()}px` }}
    >
      <div class="h-[42px] shrink-0" />

      <div class="px-3 pb-2">
        <RowButton
          onClick={props.onNewSession}
          class={`ui-rail-command ${INTERACTIVE_MOTION}`}
        >
          <SquarePen class="ui-rail-icon" />
          <span>New AppSession</span>
        </RowButton>
        <label class="ui-rail-search mt-1.5">
          <Search class="ui-rail-icon" />
          <input
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            placeholder="Search"
          />
        </label>
        <RowButton
          onClick={props.onOpenAutomations}
          class={`ui-rail-command mt-1.5 ${INTERACTIVE_MOTION}`}
        >
          <Clock3 class="ui-rail-icon" />
          <span>Automations</span>
        </RowButton>
      </div>

      <div class="flex items-center px-3 pb-1 pt-0.5">
        <div class="ui-rail-section-label">AppSessions</div>
      </div>

      <div class="flex-1 overflow-y-auto px-2 pb-2">
        <For each={filteredSessions()} fallback={
          <div class="px-3 py-8 text-center text-xs theme-muted">
            No AppSessions
          </div>
        }>
          {(session) => {
            const isActive = () => session.id === props.activeSessionId();
            return (
              <div
                role="button"
                tabindex="0"
                onClick={() => { if (renamingSessionId() !== session.id) props.setActiveSessionId(session.id); }}
                onDblClick={() => {
                  setRenamingSessionId(session.id);
                  setRenameValue(session.title);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") props.setActiveSessionId(session.id);
                }}
                class="ui-session-row group"
                classList={{ "is-active": isActive() }}
                title={session.cwd ?? session.title}
              >
                <div class="ui-session-row-inner">
                  <span class="ui-session-status-slot">
                    <Show when={isRunning(session)} fallback={
                      <span class={`ui-status-dot ${statusClass(session)}`} title={statusTitle(session)} />
                    }>
                      <span title="Running">
                        <LoaderCircle size={14} class="ui-running-spinner" />
                      </span>
                    </Show>
                  </span>
                  <Show when={renamingSessionId() === session.id} fallback={
                    <span class="ui-session-title">{session.title}</span>
                  }>
                    <input
                      class="min-w-0 flex-1 border-b border-[var(--ui-border-strong)] bg-transparent text-[13px] font-medium theme-text outline-none"
                      value={renameValue()}
                      onInput={(e) => setRenameValue(e.currentTarget.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") void commitRename(session.id);
                        if (e.key === "Escape") setRenamingSessionId(null);
                      }}
                      onBlur={() => { void commitRename(session.id); }}
                      onClick={(e) => e.stopPropagation()}
                      ref={(el) => queueMicrotask(() => el?.select())}
                    />
                  </Show>
                  <Show when={props.sessions.length > 1}>
                    <IconButton
                      size="sm"
                      class="ui-session-close"
                      title="Close AppSession"
                      onClick={(e) => { e.stopPropagation(); props.onCloseSession(session.id); }}
                    >
                      <X size={12} />
                    </IconButton>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>

      <RowButton
        onClick={props.onOpenSettings}
        class={`ui-rail-command ${INTERACTIVE_MOTION} mt-auto w-full px-3 pb-2 pt-1`}
      >
        <Settings class="ui-rail-icon" />
        <span>Settings</span>
      </RowButton>
    </aside>
  );
}
