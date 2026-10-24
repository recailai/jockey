import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import {
  Check,
  ChevronDown,
  Ellipsis,
  FolderGit2,
  LayoutDashboard,
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
  StatusDot,
  ThemeToggle,
  ToolbarButton,
} from "../ui";
import type { ThemePreference } from "../../lib/theme";
import { sessionStatus, type SessionStatus } from "../../lib/sessionStatus";
import { appSessionApi } from "../../lib/tauriApi";
import { displayTitle } from "../../lib/formatHelpers";
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
  controlCenterOpen?: Accessor<boolean>;
  onToggleControlCenter?: () => void;
  themePreference?: Accessor<ThemePreference>;
  onCycleTheme?: () => void;
};

function statusOf(session: AppSession): SessionStatus {
  return sessionStatus({
    running: session.status === "running",
    submitting: session.submitting,
    externalActive: session.externalActive,
    error: session.status === "error",
    pendingPermissions: session.pendingPermissions?.length,
    pendingUserInputs: session.pendingUserInput?.length,
  });
}

export default function SessionTopbar(props: SessionTopbarProps) {
  let chipStrip: HTMLDivElement | undefined;
  const markOverflow = () => {
    const el = chipStrip;
    if (!el) return;
    el.toggleAttribute("data-more-left", el.scrollLeft > 1);
    el.toggleAttribute("data-more-right", el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  };
  onMount(() => {
    if (!chipStrip) return;
    const ro = new ResizeObserver(markOverflow);
    ro.observe(chipStrip);
    chipStrip.addEventListener("scroll", markOverflow, { passive: true });
    onCleanup(() => ro.disconnect());
  });
  // Many sessions scroll the strip; keep the active one in view when it changes.
  createEffect(() => {
    const id = props.activeSessionId();
    if (!id || !chipStrip) return;
    queueMicrotask(() => {
      chipStrip?.querySelector<HTMLElement>(`[data-session-id="${CSS.escape(id)}"]`)
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
      markOverflow();
    });
  });
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
        <div class="flex min-w-0 shrink items-center gap-1.5 px-1">
          {/* Project Dropdown */}
          <DropdownMenu>
            <DropdownTrigger
              variant="plain"
              class="flex items-center gap-1.5 px-2 py-1 rounded-md hover:bg-[var(--ui-row-hover)] text-[var(--ui-text)] font-medium cursor-pointer transition-colors max-w-[180px] shrink-0 text-[13px]"
              title={props.currentProject?.()?.rootPath ?? "Switch Project"}
            >
              <FolderGit2 size={13} class="shrink-0" />
              <span class="truncate">{props.currentProject?.()?.name ?? "Global / Workspace"}</span>
              <ChevronDown size={11} class="theme-muted shrink-0 opacity-70" />
            </DropdownTrigger>
            <DropdownContent placement="bottom-start" class="min-w-[230px]">
              <div class="px-2 py-1 text-[10px] font-medium uppercase text-[var(--ui-text-caption)]">
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
                      <Check size={13} class="text-[var(--ui-accent)] shrink-0 ml-2" />
                    </Show>
                  </DropdownItem>
                )}
              </For>
              <DropdownSeparator />
              <Show when={props.onOpenAddProject}>
                <DropdownItem onSelect={() => props.onOpenAddProject?.()} class="text-xs py-1.5 text-[var(--ui-accent)]">
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
                  <RefreshCw size={12} class="shrink-0 mr-1.5 text-[var(--ui-accent)]" />
                  <span>Import CLI Sessions</span>
                </DropdownItem>
              </Show>
            </DropdownContent>
          </DropdownMenu>

          <span class="theme-muted select-none mx-0.5 opacity-40 shrink-0 text-xs">/</span>

          <div class="session-chip-strip" ref={(el) => (chipStrip = el)}>
            <For each={currentSessions()}>
              {(session) => {
                const isActive = () => session.id === props.activeSessionId();
                const isRenaming = () => renamingSessionId() === session.id;

                return (
                  <div
                    role="button"
                    tabindex="0"
                    class="session-chip"
                    classList={{ "is-active": isActive() }}
                    data-session-id={session.id}
                    title={displayTitle(session.title)}
                    aria-label={`Session ${displayTitle(session.title)}`}
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
                    <StatusDot status={statusOf(session)} />

                    <Show
                      when={isRenaming()}
                      fallback={
                        <span class="session-chip-title">{displayTitle(session.title)}</span>
                      }
                    >
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

                    <Show when={props.sessions.length > 1}>
                      <button
                        type="button"
                        class="session-chip-close"
                        aria-label={`Close session ${displayTitle(session.title)}`}
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
          </div>

          <button
              type="button"
              class="session-chip-add"
              onClick={props.onNewSession}
              title="New session"
              aria-label="New session"
            >
              <Plus size={14} />
            </button>

            <Show when={currentSessions().length > 5}>
              <DropdownMenu>
                <DropdownTrigger
                  variant="plain"
                  class="session-chip-add"
                  title="All sessions in project"
                >
                  <ChevronDown size={12} class="theme-muted" />
                </DropdownTrigger>
                <DropdownContent placement="bottom-start" class="min-w-[200px]">
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
                          <StatusDot status={statusOf(sess)} />
                          <span class="truncate">{displayTitle(sess.title)}</span>
                        </div>
                        <Show when={sess.id === props.activeSessionId()}>
                          <Check size={13} class="text-[var(--ui-accent)] shrink-0 ml-2" />
                        </Show>
                      </DropdownItem>
                    )}
                  </For>
                </DropdownContent>
              </DropdownMenu>
            </Show>
        </div>
      </div>

      <div class="session-topbar-drag" data-tauri-drag-region />

      <div class="session-topbar-actions" data-tauri-drag-region="false">
        <Show when={props.themePreference && props.onCycleTheme}>
          <ThemeToggle
            class="session-topbar-action"
            preference={props.themePreference!}
            onCycle={() => props.onCycleTheme?.()}
            shortcut="⌘⇧L"
          />
        </Show>
        <Show when={props.onToggleControlCenter}>
          <ToolbarButton
            class="session-topbar-action"
            title={props.controlCenterOpen?.() ? "Back to chat" : "Control center"}
            aria-label="Toggle control center"
            active={props.controlCenterOpen?.()}
            onClick={() => props.onToggleControlCenter?.()}
          >
            <LayoutDashboard size={15} stroke-width={1.75} />
          </ToolbarButton>
        </Show>
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
