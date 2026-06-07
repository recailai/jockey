import { For, Show, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import { Bot, Ellipsis, LoaderCircle, PanelRight, PanelRightClose, Plus, Search, Settings, X } from "lucide-solid";
import type { AppSession } from "../types";
import {
  DropdownContent,
  DropdownItem,
  DropdownMenu,
  DropdownSeparator,
  DropdownTrigger,
  ToolbarButton,
} from "../ui";
import { appSessionApi } from "../../lib/tauriApi";
import type { SettingsTab } from "../SettingsPage";
type SessionTopbarProps = {
  sessions: AppSession[];
  activeSessionId: Accessor<string | null>;
  setActiveSessionId: Setter<string | null>;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  onNewSession: () => void;
  onCloseSession: (id: string) => void;
  onOpenSettings: (tab?: SettingsTab) => void;
  onToggleRightDock?: () => void;
  rightDockOpen?: Accessor<boolean>;
};

function sessionStatusClass(session: AppSession): string {
  if (session.status === "error") return "ui-status-danger";
  return "ui-status-muted";
}

function sessionIsRunning(session: AppSession): boolean {
  return session.submitting || session.status === "running";
}

function sessionStatusTitle(session: AppSession): string {
  if (sessionIsRunning(session)) return "Running";
  if (session.status === "error") return "Error";
  return "Idle";
}

function isValidSessionTitle(sessions: AppSession[], sessionId: string, raw: string): string | null {
  const val = raw.trim();
  if (!val) return null;
  if (/\s/.test(val)) return null;
  if (sessions.some((s) => s.id !== sessionId && s.title.toLowerCase() === val.toLowerCase())) return null;
  return val;
}

export default function SessionTopbar(props: SessionTopbarProps) {
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");
  const [overflowOpen, setOverflowOpen] = createSignal(false);

  const commitRename = async (sessionId: string) => {
    const val = isValidSessionTitle(props.sessions, sessionId, renameValue());
    if (!val) {
      setRenamingSessionId(null);
      return;
    }
    try {
      await appSessionApi.update(sessionId, { title: val });
      props.updateSession(sessionId, { title: val });
    } catch { /* ignore */ }
    setRenamingSessionId(null);
  };

  return (
    <header class="session-topbar">
      <div class="session-topbar-content" data-tauri-drag-region="false">
        <div class="session-topbar-nav">
          <ToolbarButton
            class="session-topbar-action"
            title="Settings (⌘,)"
            aria-label="Settings"
            onClick={() => props.onOpenSettings("general")}
          >
            <Settings size={15} stroke-width={1.75} />
          </ToolbarButton>
          <ToolbarButton
            class="session-topbar-action"
            title="Automations"
            aria-label="Automations"
            onClick={() => props.onOpenSettings("automations")}
          >
            <Bot size={15} stroke-width={1.75} />
          </ToolbarButton>
        </div>
        <div class="session-chip-strip">
          <For each={props.sessions}>
            {(session) => (
              <div
                role="button"
                tabindex="0"
                class="session-chip"
                classList={{ "is-active": session.id === props.activeSessionId() }}
                title={session.cwd ?? session.title}
                aria-label={`Session ${session.title}`}
                onClick={() => {
                  if (renamingSessionId() !== session.id) props.setActiveSessionId(session.id);
                }}
                onDblClick={(e) => {
                  e.stopPropagation();
                  setRenamingSessionId(session.id);
                  setRenameValue(session.title);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") props.setActiveSessionId(session.id);
                }}
              >
                <Show when={sessionIsRunning(session)} fallback={
                  <span class={`ui-status-dot ${sessionStatusClass(session)}`} title={sessionStatusTitle(session)} />
                }>
                  <span title="Running">
                    <LoaderCircle size={13} class="ui-running-spinner" />
                  </span>
                </Show>
                <Show when={renamingSessionId() === session.id} fallback={
                  <span class="truncate">{session.title}</span>
                }>
                  <input
                    class="session-chip-rename-input"
                    value={renameValue()}
                    aria-label="Rename session"
                    onInput={(e) => setRenameValue(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Enter") void commitRename(session.id);
                      if (e.key === "Escape") setRenamingSessionId(null);
                    }}
                    onBlur={() => { void commitRename(session.id); }}
                    onClick={(e) => e.stopPropagation()}
                    ref={(el) => queueMicrotask(() => el?.select())}
                  />
                </Show>
                <Show when={session.id === props.activeSessionId() && props.sessions.length > 1}>
                  <button
                    type="button"
                    class="session-chip-close"
                    title="Close session"
                    aria-label={`Close session ${session.title}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      props.onCloseSession(session.id);
                    }}
                  >
                    <X size={12} />
                  </button>
                </Show>
              </div>
            )}
          </For>
          <button
            type="button"
            class="session-chip-add"
            onClick={props.onNewSession}
            title="New session (⌘K)"
            aria-label="New session"
          >
            <Plus size={15} />
          </button>
        </div>
      </div>

      <div class="session-topbar-drag" data-tauri-drag-region />

      <div class="session-topbar-actions" data-tauri-drag-region="false">
        <ToolbarButton
          class="session-topbar-action"
          title="Search archived sessions"
          aria-label="Search sessions"
          onClick={() => props.onOpenSettings("archived")}
        >
          <Search size={15} stroke-width={1.75} />
        </ToolbarButton>
        <Show when={props.onToggleRightDock}>
          <ToolbarButton
            class="session-topbar-action"
            title="Toggle side panel (⌘B)"
            aria-label="Toggle side panel"
            active={props.rightDockOpen?.()}
            onClick={() => props.onToggleRightDock?.()}
          >
            <Show when={props.rightDockOpen?.()} fallback={<PanelRight size={15} stroke-width={1.75} />}>
              <PanelRightClose size={15} stroke-width={1.75} />
            </Show>
          </ToolbarButton>
        </Show>
        <DropdownMenu open={overflowOpen()} onOpenChange={setOverflowOpen}>
          <DropdownTrigger
            variant="plain"
            as={ToolbarButton}
            class="session-topbar-action"
            title="More"
            aria-label="More options"
          >
            <Ellipsis size={15} stroke-width={1.75} />
          </DropdownTrigger>
          <DropdownContent placement="bottom-end" class="min-w-[200px]">
            <DropdownItem onSelect={() => { setOverflowOpen(false); props.onOpenSettings("archived"); }}>
              Archived sessions
            </DropdownItem>
            <DropdownSeparator />
            <DropdownItem onSelect={() => { setOverflowOpen(false); props.onOpenSettings("roles"); }}>
              Roles
            </DropdownItem>
            <DropdownItem onSelect={() => { setOverflowOpen(false); props.onOpenSettings("mcp"); }}>
              MCP servers
            </DropdownItem>
          </DropdownContent>
        </DropdownMenu>
      </div>
    </header>
  );
}
