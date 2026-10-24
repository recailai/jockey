import { For, Show, createMemo, createSignal, onMount, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import {
  ChevronDown,
  ChevronRight,
  Copy,
  Edit3,
  Files,
  FolderGit2,
  GitBranch,
  Download,
  FolderPlus,
  Plus,
  Settings,
  SquareTerminal,
  ArchiveRestore,
  Bot,
  PanelLeftClose,
  Trash2,
  X,
} from "lucide-solid";
import type { AppSession } from "../types";
import type { Project } from "../../lib/tauriApi";
import { appSessionApi } from "../../lib/tauriApi";
import { Button, ContextMenuSurface, ContextMenuItem, ContextMenuSeparator, IconButton, RowButton, StatusDot } from "../ui";
import { sessionStatus, type SessionStatus } from "../../lib/sessionStatus";
import { displayTitle } from "../../lib/formatHelpers";

type ProjectSessionSidebarProps = {
  projects: Accessor<Project[]>;
  currentProject: Accessor<Project | null>;
  sessions: Accessor<AppSession[]>;
  activeSessionId: Accessor<string | null>;
  onSelectProject: (p: Project) => Promise<void>;
  onSelectSession: (sessionId: string, project?: Project | null) => void;
  onNewSession: (projectId?: string) => void;
  onCloseSession: (sessionId: string) => void;
  onDuplicateSession?: (session: AppSession) => void;
  onForkSession?: (session: AppSession) => void;
  canForkSession?: (session: AppSession) => boolean;
  onOpenSessionInTerminal?: (session: AppSession) => void;
  onDeleteProject?: (id: string) => Promise<void>;
  onOpenAddProject: () => void;
  onImportSessions?: () => void;
  onOpenSettings: (tab?: string) => void;
  onToggleSidebar: () => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
};

const COLLAPSED_STORAGE_KEY = "jockey.sidebar.collapsedProjects";

function getInitialCollapsed(): Set<string> {
  try {
    const raw = window.localStorage.getItem(COLLAPSED_STORAGE_KEY);
    return raw ? new Set(JSON.parse(raw)) : new Set();
  } catch {
    return new Set();
  }
}

export default function ProjectSessionSidebar(props: ProjectSessionSidebarProps) {
  const [collapsedProjects, setCollapsedProjects] = createSignal<Set<string>>(getInitialCollapsed());
  const [renamingSessionId, setRenamingSessionId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");

  const [projectContextMenu, setProjectContextMenu] = createSignal<{ x: number; y: number; project: Project } | null>(null);
  const [sessionContextMenu, setSessionContextMenu] = createSignal<{ x: number; y: number; session: AppSession; project?: Project | null } | null>(null);

  const closeContextMenus = () => {
    setProjectContextMenu(null);
    setSessionContextMenu(null);
  };

  onMount(() => {
    const handleGlobalClick = () => closeContextMenus();
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeContextMenus();
    };
    window.addEventListener("pointerdown", handleGlobalClick);
    window.addEventListener("keydown", handleKeyDown);
    onCleanup(() => {
      window.removeEventListener("pointerdown", handleGlobalClick);
      window.removeEventListener("keydown", handleKeyDown);
    });
  });

  const toggleProject = (projectId: string) => {
    setCollapsedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(projectId)) next.delete(projectId);
      else next.add(projectId);
      try {
        window.localStorage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify(Array.from(next)));
      } catch { /* ignore */ }
      return next;
    });
  };

  const isCollapsed = (projectId: string) => collapsedProjects().has(projectId);

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

  const sessionsByProject = createMemo(() => {
    const map = new Map<string, AppSession[]>();
    const unassigned: AppSession[] = [];
    const all = props.sessions();

    for (const s of all) {
      if (s.projectId) {
        const list = map.get(s.projectId) ?? [];
        list.push(s);
        map.set(s.projectId, list);
      } else {
        unassigned.push(s);
      }
    }
    return { map, unassigned };
  });

  const statusOf = (session: AppSession): SessionStatus =>
    sessionStatus({
      running: session.status === "running",
      submitting: session.submitting,
      externalActive: session.externalActive,
      error: session.status === "error",
      pendingPermissions: session.pendingPermissions?.length,
      pendingUserInputs: session.pendingUserInput?.length,
    });

  const sessionRow = (session: AppSession, project: Project | null) => {
    const isActive = () => session.id === props.activeSessionId();
    const renaming = () => renamingSessionId() === session.id;
    return (
      <div
        role="button"
        tabindex="0"
        aria-current={isActive() ? "true" : undefined}
        onClick={() => props.onSelectSession(session.id, project)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onSelectSession(session.id, project);
          }
        }}
        onDblClick={(e) => {
          e.stopPropagation();
          setRenamingSessionId(session.id);
          setRenameValue(session.title);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setProjectContextMenu(null);
          setSessionContextMenu({ x: e.clientX, y: e.clientY, session, project });
        }}
        class="sidebar-session-row"
        classList={{ "is-active": isActive() }}
        title={session.cwd ?? session.title}
      >
        <span class="sidebar-row-slot">
          <StatusDot status={statusOf(session)} />
        </span>
        <Show
          when={renaming()}
          fallback={<span class="sidebar-row-title">{displayTitle(session.title)}</span>}
        >
          <input
            type="text"
            autofocus
            value={renameValue()}
            onInput={(e) => setRenameValue(e.currentTarget.value)}
            onBlur={() => void commitRename(session.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commitRename(session.id);
              if (e.key === "Escape") setRenamingSessionId(null);
            }}
            onClick={(e) => e.stopPropagation()}
            class="jui-field sidebar-rename-input"
          />
        </Show>
        <Show when={!renaming()}>
          <span class="sidebar-row-actions" onClick={(e) => e.stopPropagation()}>
            <IconButton
              size="sm"
              title="Close session"
              aria-label={`Close session ${session.title}`}
              onClick={() => props.onCloseSession(session.id)}
            >
              <X size={12} />
            </IconButton>
          </span>
        </Show>
      </div>
    );
  };

  return (
    <aside class="jockey-tree-sidebar sidebar-shell">
      {/* ── Header ── */}
      <div class="sidebar-header">
        <span class="flex-1" />
        <div class="flex items-center gap-0.5">
          <Show when={props.onImportSessions}>
            <IconButton
              size="sm"
              onClick={props.onImportSessions}
              title="Import CLI sessions"
              aria-label="Import CLI sessions"
            >
              <Download size={14} />
            </IconButton>
          </Show>
          <IconButton
            size="sm"
            onClick={props.onOpenAddProject}
            title="Add Project"
            aria-label="Add Project"
          >
            <FolderPlus size={14} />
          </IconButton>
          <IconButton
            size="sm"
            onClick={props.onToggleSidebar}
            title="Collapse Sidebar (⌘B)"
            aria-label="Collapse Sidebar (⌘B)"
          >
            <PanelLeftClose size={14} />
          </IconButton>
        </div>
      </div>

      {/* ── Project & Session Tree Body ── */}
      <div class="sidebar-tree">
        <Show when={props.projects().length === 0}>
          <div class="sidebar-empty">
            <p>No projects added yet.</p>
            <Button variant="outline" size="sm" onClick={props.onOpenAddProject}>
              Add project
            </Button>
          </div>
        </Show>

        <For each={props.projects()}>
          {(project) => {
            const isCurrent = () => props.currentProject()?.id === project.id;
            const collapsed = () => isCollapsed(project.id);
            const projectSessions = () => sessionsByProject().map.get(project.id) ?? [];

            return (
              <div class="project-tree-node">
                <div
                  role="button"
                  tabindex="0"
                  aria-expanded={!collapsed()}
                  onClick={() => {
                    if (!isCurrent()) void props.onSelectProject(project);
                    toggleProject(project.id);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      toggleProject(project.id);
                    }
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setSessionContextMenu(null);
                    setProjectContextMenu({ x: e.clientX, y: e.clientY, project });
                  }}
                  class="sidebar-project-row"
                  classList={{ "is-current": isCurrent() }}
                  title={project.rootPath}
                >
                  <span class="sidebar-row-slot sidebar-project-icon">
                    <Show when={collapsed()} fallback={<ChevronDown size={12} />}>
                      <ChevronRight size={12} />
                    </Show>
                  </span>
                  <FolderGit2 size={14} class="sidebar-folder-icon" />
                  <span class="sidebar-row-title">{project.name}</span>
                  <Show when={projectSessions().length > 0}>
                    <span class="sidebar-row-count">{projectSessions().length}</span>
                  </Show>
                  <span class="sidebar-row-actions" onClick={(e) => e.stopPropagation()}>
                    <IconButton
                      size="sm"
                      title="New session in this project"
                      aria-label={`New session in ${project.name}`}
                      onClick={() => {
                        if (!isCurrent()) void props.onSelectProject(project);
                        props.onNewSession(project.id);
                      }}
                    >
                      <Plus size={12} />
                    </IconButton>
                  </span>
                </div>

                <Show when={!collapsed()}>
                  <div class="sidebar-session-list">
                    <Show
                      when={projectSessions().length > 0}
                      fallback={
                        <RowButton
                          class="sidebar-new-session"
                          onClick={() => {
                            if (!isCurrent()) void props.onSelectProject(project);
                            props.onNewSession(project.id);
                          }}
                        >
                          <Plus size={12} />
                          New session
                        </RowButton>
                      }
                    >
                      <For each={projectSessions()}>{(session) => sessionRow(session, project)}</For>
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>

        {/* Unassigned / Global Sessions node if any */}
        <Show when={sessionsByProject().unassigned.length > 0}>
          <div class="project-tree-node sidebar-unassigned">
            <div
              role="button"
              tabindex="0"
              aria-expanded={!isCollapsed("__unassigned__")}
              onClick={() => toggleProject("__unassigned__")}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  toggleProject("__unassigned__");
                }
              }}
              class="sidebar-project-row"
            >
              <span class="sidebar-row-slot sidebar-project-icon">
                <Show when={isCollapsed("__unassigned__")} fallback={<ChevronDown size={12} />}>
                  <ChevronRight size={12} />
                </Show>
              </span>
              <span class="sidebar-row-title">Other sessions</span>
              <span class="sidebar-row-count">{sessionsByProject().unassigned.length}</span>
            </div>
            <Show when={!isCollapsed("__unassigned__")}>
              <div class="sidebar-session-list">
                <For each={sessionsByProject().unassigned}>{(session) => sessionRow(session, null)}</For>
              </div>
            </Show>
          </div>
        </Show>
      </div>

      {/* ── Footer ── */}
      <div class="sidebar-footer">
        <div class="flex min-w-0 items-center gap-0.5">
          <Button
            variant="ghost"
            size="sm"
            class="sidebar-footer-button"
            onClick={() => props.onOpenSettings("roles")}
            title="Configure Roles"
          >
            <Bot size={13} />
            <span>Roles</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            class="sidebar-footer-button"
            onClick={() => props.onOpenSettings("sessions")}
            title="Closed Sessions (Recycle Bin)"
          >
            <ArchiveRestore size={13} />
            <span>Trash</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            class="sidebar-footer-button"
            onClick={() => props.onOpenSettings("general")}
            title="Settings (⌘,)"
          >
            <Settings size={13} />
            <span>Settings</span>
          </Button>
        </div>
      </div>
      {/* ── Project Context Menu ── */}
      <Show when={projectContextMenu()}>
        {(menu) => (
          <ContextMenuSurface
            x={menu().x}
            y={menu().y}
            width={190}
            onClick={(e) => e.stopPropagation()}
          >
            <ContextMenuItem
              icon={<Plus size={13} />}
              onSelect={() => {
                closeContextMenus();
                if (props.currentProject()?.id !== menu().project.id) {
                  void props.onSelectProject(menu().project);
                }
                props.onNewSession(menu().project.id);
              }}
            >
              New Session
            </ContextMenuItem>
            <ContextMenuItem
              icon={<Copy size={13} />}
              onSelect={() => {
                closeContextMenus();
                void navigator.clipboard.writeText(menu().project.rootPath);
              }}
            >
              Copy Root Path
            </ContextMenuItem>
            <ContextMenuItem
              icon={<Settings size={13} />}
              onSelect={() => {
                closeContextMenus();
                props.onOpenSettings("roles");
              }}
            >
              Project Roles
            </ContextMenuItem>
            <ContextMenuSeparator />
            <Show when={props.onDeleteProject}>
              <ContextMenuItem
                icon={<Trash2 size={13} class="text-[var(--ui-state-danger-text)]" />}
                class="text-[var(--ui-state-danger-text)] hover:text-[var(--ui-state-danger-text)]"
                onSelect={() => {
                  const pid = menu().project.id;
                  closeContextMenus();
                  if (confirm(`Are you sure you want to remove project '${menu().project.name}' from Jockey?`)) {
                    void props.onDeleteProject?.(pid);
                  }
                }}
              >
                Delete Project
              </ContextMenuItem>
            </Show>
          </ContextMenuSurface>
        )}
      </Show>

      {/* ── Session Context Menu ── */}
      <Show when={sessionContextMenu()}>
        {(menu) => (
          <ContextMenuSurface
            x={menu().x}
            y={menu().y}
            width={180}
            onClick={(e) => e.stopPropagation()}
          >
            <ContextMenuItem
              icon={<Edit3 size={13} />}
              onSelect={() => {
                const s = menu().session;
                closeContextMenus();
                setRenamingSessionId(s.id);
                setRenameValue(s.title);
              }}
            >
              Rename
            </ContextMenuItem>
            <Show when={props.onDuplicateSession}>
              <ContextMenuItem
                icon={<Files size={13} />}
                onSelect={() => {
                  const s = menu().session;
                  closeContextMenus();
                  props.onDuplicateSession?.(s);
                }}
              >
                Duplicate
              </ContextMenuItem>
            </Show>
            <Show when={props.onForkSession && props.canForkSession?.(menu().session)}>
              <ContextMenuItem
                icon={<GitBranch size={13} />}
                onSelect={() => {
                  const s = menu().session;
                  closeContextMenus();
                  props.onForkSession?.(s);
                }}
              >
                Fork
              </ContextMenuItem>
            </Show>
            <Show when={props.onOpenSessionInTerminal}>
              <ContextMenuItem
                icon={<SquareTerminal size={13} />}
                onSelect={() => {
                  const s = menu().session;
                  closeContextMenus();
                  props.onOpenSessionInTerminal?.(s);
                }}
              >
                Open in Terminal
              </ContextMenuItem>
            </Show>
            <ContextMenuSeparator />
            <ContextMenuItem
              icon={<Trash2 size={13} class="text-[var(--ui-state-danger-text)]" />}
              class="text-[var(--ui-state-danger-text)] hover:text-[var(--ui-state-danger-text)]"
              onSelect={() => {
                const sid = menu().session.id;
                closeContextMenus();
                props.onCloseSession(sid);
              }}
            >
              Close Session
            </ContextMenuItem>
          </ContextMenuSurface>
        )}
      </Show>
    </aside>
  );
}
