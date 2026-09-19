import { For, Show, createMemo, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import {
  Check,
  ChevronDown,
  Ellipsis,
  FolderGit2,
  LoaderCircle,
  PanelLeft,
  PanelRight,
  PanelRightClose,
  Plus,
  RefreshCw,
  Search,
  X,
} from "lucide-solid";
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
import type { Project } from "../../lib/tauriApi";

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
  leftSidebarOpen?: Accessor<boolean>;
  onToggleLeftSidebar?: () => void;
  currentProject?: Accessor<Project | null>;
  projects?: Accessor<Project[]>;
  onSelectProject?: (p: Project) => Promise<void>;
  onSelectSession?: (sessionId: string, project?: Project | null) => void;
  onOpenAddProject?: () => void;
  onImportSessions?: () => void;
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

export default function SessionTopbar(props: SessionTopbarProps) {
  const [overflowOpen, setOverflowOpen] = createSignal(false);
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");

  const commitRename = async (sessionId: string) => {
    const val = renameValue().trim();
    if (!val || /\s/.test(val)) {
      setRenamingSessionId(null);
      return;
    }
    try {
      await appSessionApi.update(sessionId, { title: val });
      props.updateSession(sessionId, { title: val });
    } catch { /* ignore */ }
    setRenamingSessionId(null);
  };

  const currentSessions = createMemo(() => {
    const curPid = props.currentProject?.()?.id;
    if (!curPid) return props.sessions;
    return props.sessions.filter((s) => s.projectId === curPid);
  });

  return (
    <header class="session-topbar">
      <div class="session-topbar-content" data-tauri-drag-region="false">
        <div class="session-topbar-nav flex items-center gap-1.5">
          <ToolbarButton
            class="session-topbar-action"
            title={props.leftSidebarOpen?.() ? "Collapse sidebar (⌘B)" : "Expand sidebar (⌘B)"}
            aria-label="Toggle Sidebar"
            onClick={() => props.onToggleLeftSidebar?.()}
          >
            <PanelLeft size={15} stroke-width={1.75} class={props.leftSidebarOpen?.() ? "text-[var(--ui-accent)]" : "theme-muted hover:text-[var(--ui-text)]"} />
          </ToolbarButton>
        </div>

        <div class="session-topbar-divider mx-1 shrink-0" />

        {/* ── Project Switcher & Horizontal Sessions Strip ── */}
        <div class="flex items-center gap-1.5 px-1 min-w-0 flex-1">
          {/* Project Dropdown */}
          <DropdownMenu>
            <DropdownTrigger
              variant="plain"
              class="flex items-center gap-1.5 px-2 py-1 rounded-md hover:bg-[var(--ui-surface-muted)] text-blue-400 font-semibold cursor-pointer transition-colors max-w-[180px] shrink-0 font-mono text-xs"
              title={props.currentProject?.()?.rootPath ?? "Switch Project"}
            >
              <FolderGit2 size={13} class="shrink-0" />
              <span class="truncate">{props.currentProject?.()?.name ?? "Global / Workspace"}</span>
              <ChevronDown size={11} class="theme-muted shrink-0 opacity-70" />
            </DropdownTrigger>
            <DropdownContent placement="bottom-start" class="min-w-[230px]">
              <div class="px-2 py-1 text-[10px] font-mono font-bold uppercase theme-muted">
                Projects
              </div>
              <For each={props.projects?.() ?? []}>
                {(proj) => (
                  <DropdownItem
                    onSelect={() => {
                      if (proj.id !== props.currentProject?.()?.id) {
                        void props.onSelectProject?.(proj);
                      }
                    }}
                    class="flex items-center justify-between text-xs py-1.5"
                  >
                    <div class="flex items-center gap-1.5 truncate">
                      <FolderGit2 size={13} class="theme-muted shrink-0" />
                      <span class="truncate">{proj.name}</span>
                    </div>
                    <Show when={proj.id === props.currentProject?.()?.id}>
                      <Check size={13} class="text-blue-400 shrink-0 ml-2" />
                    </Show>
                  </DropdownItem>
                )}
              </For>
              <DropdownSeparator />
              <Show when={props.onOpenAddProject}>
                <DropdownItem onSelect={() => props.onOpenAddProject?.()} class="text-xs py-1.5 text-blue-400">
                  <Plus size={13} class="shrink-0 mr-1.5" />
                  <span>Add Project...</span>
                </DropdownItem>
              </Show>
              <Show when={props.currentProject?.() && props.onImportSessions}>
                <DropdownItem
                  onSelect={() => {
                    props.onImportSessions?.();
                  }}
                  class="text-xs py-1.5"
                >
                  <RefreshCw size={12} class="shrink-0 mr-1.5 text-blue-400" />
                  <span>Import CLI Sessions</span>
                </DropdownItem>
              </Show>
            </DropdownContent>
          </DropdownMenu>

          <span class="theme-muted select-none mx-0.5 opacity-40 shrink-0 font-mono text-xs">/</span>

          {/* ── Horizontal Session Chips (同项目不同session横向排列 & 可操控) ── */}
          <div class="session-chip-strip flex-1 min-w-0 flex items-center">
            <For each={currentSessions()}>
              {(session) => {
                const isRunning = () => sessionIsRunning(session);
                const isActive = () => session.id === props.activeSessionId();
                const isRenaming = () => renamingSessionId() === session.id;

                return (
                  <div
                    role="button"
                    tabindex="0"
                    class="session-chip group relative cursor-pointer flex-shrink min-w-0"
                    classList={{ "is-active": isActive() }}
                    title={session.cwd ?? session.title}
                    aria-label={`Session ${session.title}`}
                    onClick={() => {
                      if (!isRenaming()) {
                        if (props.onSelectSession) {
                          props.onSelectSession(session.id, props.currentProject?.());
                        } else {
                          props.setActiveSessionId(session.id);
                        }
                      }
                    }}
                    onDblClick={(e) => {
                      e.stopPropagation();
                      setRenamingSessionId(session.id);
                      setRenameValue(session.title);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        props.setActiveSessionId(session.id);
                      }
                    }}
                  >
                    <Show
                      when={isRunning()}
                      fallback={
                        <span
                          class={`ui-status-dot ${sessionStatusClass(session)}`}
                          title={sessionStatusTitle(session)}
                        />
                      }
                    >
                      <LoaderCircle size={12} class="ui-running-spinner text-amber-400 shrink-0" />
                    </Show>

                    <Show
                      when={isRenaming()}
                      fallback={
                        <span
                          class="truncate select-none text-xs font-mono"
                          title={session.title}
                        >
                          {session.title}
                        </span>
                      }
                    >
                      <input
                        class="session-chip-rename-input font-mono text-xs"
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

                    <Show when={props.sessions.length > 1}>
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
                        <X size={11} />
                      </button>
                    </Show>
                  </div>
                );
              }}
            </For>

            <button
              type="button"
              class="session-chip-add ml-0.5 shrink-0"
              onClick={props.onNewSession}
              title="New session in current project"
              aria-label="New session"
            >
              <Plus size={14} />
            </button>

            <Show when={currentSessions().length > 5}>
              <DropdownMenu>
                <DropdownTrigger
                  variant="plain"
                  class="session-chip-add shrink-0 ml-0.5"
                  title="All sessions in project"
                >
                  <ChevronDown size={12} class="theme-muted" />
                </DropdownTrigger>
                <DropdownContent placement="bottom-start" class="min-w-[200px]">
                  <div class="px-2 py-1 text-[10px] font-mono font-bold uppercase theme-muted">
                    All Sessions ({currentSessions().length})
                  </div>
                  <For each={currentSessions()}>
                    {(sess) => (
                      <DropdownItem
                        onSelect={() => {
                          if (props.onSelectSession) {
                            props.onSelectSession(sess.id, props.currentProject?.());
                          } else {
                            props.setActiveSessionId(sess.id);
                          }
                        }}
                        class="flex items-center justify-between text-xs py-1.5"
                      >
                        <div class="flex items-center gap-1.5 truncate">
                          <span class={`ui-status-dot ${sessionStatusClass(sess)}`} />
                          <span class="truncate">{sess.title}</span>
                        </div>
                        <Show when={sess.id === props.activeSessionId()}>
                          <Check size={13} class="text-blue-400 shrink-0 ml-2" />
                        </Show>
                      </DropdownItem>
                    )}
                  </For>
                </DropdownContent>
              </DropdownMenu>
            </Show>
          </div>
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
            title="Toggle right dock (⌘⇧B)"
            aria-label="Toggle right dock"
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
