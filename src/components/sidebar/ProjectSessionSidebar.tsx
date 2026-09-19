import { For, Show, createMemo, createSignal, onMount, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import {
  ChevronDown,
  ChevronRight,
  Copy,
  Edit3,
  Files,
  FolderGit2,
  FolderPlus,
  LoaderCircle,
  Plus,
  Settings,
  ArchiveRestore,
  Bot,
  PanelLeftClose,
  Trash2,
  X,
} from "lucide-solid";
import type { AppSession } from "../types";
import type { Project } from "../../lib/tauriApi";
import { appSessionApi } from "../../lib/tauriApi";
import { ContextMenuSurface, ContextMenuItem, ContextMenuSeparator } from "../ui";

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
  onDeleteProject?: (id: string) => Promise<void>;
  onOpenAddProject: () => void;
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

  return (
    <aside class="jockey-tree-sidebar flex flex-col w-60 shrink-0 border-r theme-border bg-[var(--ui-surface)] select-none text-xs">
      {/* ── Header ── */}
      <div class="flex items-center justify-between px-3 h-10 border-b theme-border">
        <span class="font-mono text-[11px] font-bold uppercase tracking-wider theme-text">
          Projects
        </span>
        <div class="flex items-center gap-0.5">
          <button
            type="button"
            onClick={props.onOpenAddProject}
            title="Add Project"
            class="p-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)] transition-colors"
          >
            <FolderPlus size={14} />
          </button>
          <button
            type="button"
            onClick={props.onToggleSidebar}
            title="Collapse Sidebar (⌘B)"
            class="p-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)] transition-colors"
          >
            <PanelLeftClose size={14} />
          </button>
        </div>
      </div>

      {/* ── Project & Session Tree Body ── */}
      <div class="flex-1 overflow-y-auto p-1.5 space-y-1">
        <Show when={props.projects().length === 0}>
          <div class="p-4 text-center">
            <p class="text-[11px] theme-muted mb-2">No projects added yet.</p>
            <button
              type="button"
              onClick={props.onOpenAddProject}
              class="px-2.5 py-1 text-[11px] font-medium rounded border border-[var(--ui-border)] hover:bg-[var(--ui-surface-muted)] theme-text transition-colors"
            >
              + Add Project
            </button>
          </div>
        </Show>

        <For each={props.projects()}>
          {(project) => {
            const isCurrent = () => props.currentProject()?.id === project.id;
            const collapsed = () => isCollapsed(project.id);
            const projectSessions = () => sessionsByProject().map.get(project.id) ?? [];

            return (
              <div class="project-tree-node rounded-lg overflow-hidden transition-colors">
                {/* Project Header Row */}
                <div
                  role="button"
                  tabindex="0"
                  onClick={() => {
                    if (!isCurrent()) void props.onSelectProject(project);
                    toggleProject(project.id);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setSessionContextMenu(null);
                    setProjectContextMenu({ x: e.clientX, y: e.clientY, project });
                  }}
                  class={`group flex items-center gap-1.5 px-2 py-1.5 rounded-md cursor-pointer transition-colors ${
                    isCurrent()
                      ? "bg-[var(--ui-surface-muted)] text-[var(--ui-text)] font-semibold"
                      : "hover:bg-[var(--ui-surface-muted)] theme-muted hover:theme-text"
                  }`}
                  title={project.rootPath}
                >
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleProject(project.id);
                    }}
                    class="p-0.5 theme-muted hover:theme-text transition-transform"
                  >
                    <Show when={collapsed()} fallback={<ChevronDown size={12} />}>
                      <ChevronRight size={12} />
                    </Show>
                  </button>

                  <FolderGit2
                    size={14}
                    class={isCurrent() ? "text-blue-500 shrink-0" : "theme-muted shrink-0"}
                  />

                  <span class="truncate flex-1 min-w-0 font-medium text-[11.5px]">
                    {project.name}
                  </span>

                  {/* Actions on hover */}
                  <div
                    class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      onClick={() => {
                        if (!isCurrent()) void props.onSelectProject(project);
                        props.onNewSession(project.id);
                      }}
                      title="New Session in this Project"
                      class="p-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface)] transition-colors"
                    >
                      <Plus size={12} />
                    </button>
                  </div>
                </div>

                {/* Nested Sessions List */}
                <Show when={!collapsed()}>
                  <div class="ml-4 pl-2 border-l border-[var(--ui-border)] mt-0.5 space-y-0.5 py-0.5">
                    <Show
                      when={projectSessions().length > 0}
                      fallback={
                        <button
                          type="button"
                          onClick={() => {
                            if (!isCurrent()) void props.onSelectProject(project);
                            props.onNewSession(project.id);
                          }}
                          class="block w-full text-left px-2 py-1 text-[10.5px] theme-muted italic hover:theme-text hover:underline"
                        >
                          + New session
                        </button>
                      }
                    >
                      <For each={projectSessions()}>
                        {(session) => {
                          const isActive = () => session.id === props.activeSessionId();
                          const isRunning = () => session.submitting || session.status === "running";
                          const isNative = () =>
                            session.runtimeKind?.includes("native") ||
                            session.runtimeKind === "antigravity-cli" ||
                            session.runtimeKind === "codex-cli" ||
                            session.runtimeKind === "pi-cli";

                          return (
                            <div
                              role="button"
                              tabindex="0"
                              onClick={() => props.onSelectSession(session.id, project)}
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
                              class={`group/session flex items-center justify-between gap-1.5 px-2 py-1 rounded cursor-pointer transition-colors text-[11px] ${
                                isActive()
                                  ? "bg-[var(--ui-accent-muted)] text-[var(--ui-accent)] font-medium"
                                  : "hover:bg-[var(--ui-surface-muted)] theme-text"
                              }`}
                              title={session.cwd ?? session.title}
                            >
                              <div class="flex items-center gap-1.5 min-w-0 flex-1">
                                <Show
                                  when={isRunning()}
                                  fallback={
                                    <span
                                      class={`w-1.5 h-1.5 rounded-full shrink-0 ${
                                        session.status === "error"
                                          ? "bg-rose-400"
                                          : isActive()
                                          ? "bg-[var(--ui-accent)]"
                                          : "bg-[var(--ui-muted)] opacity-50"
                                      }`}
                                    />
                                  }
                                >
                                  <LoaderCircle size={10} class="animate-spin text-[var(--ui-accent)] shrink-0" />
                                </Show>

                                <Show
                                  when={renamingSessionId() === session.id}
                                  fallback={
                                    <span class="truncate font-mono text-[11px]">
                                      {session.title}
                                    </span>
                                  }
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
                                    class="h-5 w-full bg-[var(--ui-surface)] border border-[var(--ui-border)] px-1 rounded text-[11px] font-mono theme-text outline-none"
                                  />
                                </Show>
                              </div>

                              {/* Right side: Role/Mode badge or Close button */}
                              <div class="flex items-center gap-1 shrink-0">
                                <Show
                                  when={renamingSessionId() !== session.id}
                                >
                                  <span class="font-mono text-[8.5px] px-1 py-0.2 rounded border border-[var(--ui-border)] opacity-60 group-hover/session:opacity-0 transition-opacity">
                                    {isNative() ? "⚡" : "🔌"}
                                  </span>
                                  <button
                                    type="button"
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      props.onCloseSession(session.id);
                                    }}
                                    title="Close Session"
                                    class="opacity-0 group-hover/session:opacity-100 p-0.5 rounded theme-muted hover:text-rose-400 hover:bg-[var(--ui-surface)] transition-all"
                                  >
                                    <X size={10} />
                                  </button>
                                </Show>
                              </div>
                            </div>
                          );
                        }}
                      </For>
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>

        {/* Unassigned / Global Sessions node if any */}
        <Show when={sessionsByProject().unassigned.length > 0}>
          <div class="project-tree-node rounded-lg overflow-hidden transition-colors mt-2 pt-2 border-t theme-border">
            <div
              role="button"
              tabindex="0"
              onClick={() => toggleProject("__unassigned__")}
              class="flex items-center gap-1.5 px-2 py-1.5 rounded-md cursor-pointer theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)] transition-colors"
            >
              <Show when={isCollapsed("__unassigned__")} fallback={<ChevronDown size={12} />}>
                <ChevronRight size={12} />
              </Show>
              <span class="font-mono text-[11px] font-medium truncate flex-1">Global / Other</span>
            </div>
            <Show when={!isCollapsed("__unassigned__")}>
              <div class="ml-4 pl-2 border-l border-[var(--ui-border)] mt-0.5 space-y-0.5 py-0.5">
                <For each={sessionsByProject().unassigned}>
                  {(session) => (
                    <div
                      role="button"
                      tabindex="0"
                      onClick={() => props.onSelectSession(session.id, null)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setProjectContextMenu(null);
                        setSessionContextMenu({ x: e.clientX, y: e.clientY, session, project: null });
                      }}
                      class={`flex items-center justify-between gap-1.5 px-2 py-1 rounded cursor-pointer transition-colors text-[11px] ${
                        session.id === props.activeSessionId()
                          ? "bg-[var(--ui-accent-muted)] text-[var(--ui-accent)] font-medium"
                          : "hover:bg-[var(--ui-surface-muted)] theme-text"
                      }`}
                    >
                      <span class="truncate font-mono text-[11px]">{session.title}</span>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          props.onCloseSession(session.id);
                        }}
                        class="p-0.5 rounded theme-muted hover:text-rose-400"
                      >
                        <X size={10} />
                      </button>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
      </div>

      {/* ── Footer ── */}
      <div class="p-2 border-t theme-border flex items-center justify-between text-xs bg-[var(--ui-surface-muted)]">
        <div class="flex items-center gap-1">
          <button
            type="button"
            onClick={() => props.onOpenSettings("roles")}
            title="Configure Roles"
            class="flex items-center gap-1 px-1.5 py-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface)] transition-colors text-[11px]"
          >
            <Bot size={13} />
            <span>Roles</span>
          </button>
          <button
            type="button"
            onClick={() => props.onOpenSettings("sessions")}
            title="Closed Sessions (Recycle Bin)"
            class="flex items-center gap-1 px-1.5 py-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface)] transition-colors text-[11px]"
          >
            <ArchiveRestore size={13} />
            <span>Trash</span>
          </button>
          <button
            type="button"
            onClick={() => props.onOpenSettings("general")}
            title="Settings (⌘,)"
            class="flex items-center gap-1 px-1.5 py-1 rounded theme-muted hover:theme-text hover:bg-[var(--ui-surface)] transition-colors text-[11px]"
          >
            <Settings size={13} />
            <span>Settings</span>
          </button>
        </div>
        <span class="text-[9.5px] font-mono theme-muted pr-1 opacity-60">⌘B</span>
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
                icon={<Trash2 size={13} class="text-red-400" />}
                class="text-red-400 hover:text-red-300"
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
            <ContextMenuSeparator />
            <ContextMenuItem
              icon={<Trash2 size={13} class="text-red-400" />}
              class="text-red-400 hover:text-red-300"
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
