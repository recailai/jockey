import { For, Show, createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { ArrowLeft, ChevronRight, FolderPlus, History, Plus, RefreshCw, DownloadCloud } from "lucide-solid";
import type { SessionSummary } from "../components/types";
import type { Project } from "../lib/tauriApi";
import { displayTitle, fmtRelative } from "../lib/formatHelpers";
import { AsyncContent, Badge, Button, IconButton, RowButton, StatusBadge, StatusDot } from "../components/ui";
import { sessionStatus } from "../lib/sessionStatus";

const OPEN_PROJECTS_KEY = "jockey_open_projects";
const UNASSIGNED = "__none";

function loadOpenProjects(): Set<string> {
  try {
    return new Set(JSON.parse(window.localStorage.getItem(OPEN_PROJECTS_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

type SessionListProps = {
  projects: () => Project[];
  sessions: () => SessionSummary[];
  state: () => "loading" | "ready" | "error";
  error: () => string | null;
  onRefresh: () => void;
  onOpen: (sessionId: string) => void;
  onNewSession: (projectId: string) => void;
  onShowClosed: (project: Project) => void;
  onImport: (project: Project) => void;
  onImportAll: () => void;
  onAddProject: () => void;
  onControlCenter?: () => void;
  headerActions?: JSX.Element;
};

function SessionRow(props: { session: SessionSummary; onOpen: (id: string) => void }) {
  const s = () => props.session;
  const status = () => sessionStatus({ running: s().running, externalActive: s().externalActive, error: s().lastError });
  return (
    <RowButton class="companion-session-row" onClick={() => props.onOpen(s().id)}>
      <span class="flex w-full min-w-0 flex-col gap-0.5 text-left">
        <span class="flex items-center gap-2">
          <StatusDot status={status()} />
          <span class="min-w-0 flex-1 truncate text-sm font-medium">{displayTitle(s().title)}</span>
          <span class="companion-session-time">{fmtRelative(s().lastActiveAt)}</span>
        </span>
        <Show when={s().preview}>
          <span class="companion-session-preview">{s().preview}</span>
        </Show>
        <span class="flex items-center gap-1.5">
          <Badge tone="neutral">@{s().activeRole}</Badge>
          <StatusBadge status={status()} quiet />
        </span>
      </span>
    </RowButton>
  );
}

export default function SessionList(props: SessionListProps) {
  const [open, setOpen] = createSignal(loadOpenProjects());

  const toggle = (id: string) => {
    const next = new Set(open());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setOpen(next);
    try {
      window.localStorage.setItem(OPEN_PROJECTS_KEY, JSON.stringify([...next]));
    } catch { /* the open state just resets next visit */ }
  };

  const byProject = (id: string) => props.sessions().filter((s) => s.projectId === id);
  const unassigned = () => {
    const known = new Set(props.projects().map((p) => p.id));
    return props.sessions().filter((s) => !s.projectId || !known.has(s.projectId));
  };
  const isOpen = (id: string) => open().has(id) || (open().size === 0 && props.projects()[0]?.id === id);

  const state = () => {
    if (props.state() === "ready" && props.projects().length === 0 && props.sessions().length === 0) return "empty";
    return props.state();
  };

  return (
    <div class="flex min-h-0 flex-1 flex-col">
      <header class="companion-header">
        <Show when={props.onControlCenter}><IconButton title="Control center" aria-label="Back to control center" onClick={props.onControlCenter}><ArrowLeft size={18} /></IconButton></Show>
        <h1 class="flex-1 text-sm font-semibold">Sessions</h1>
        {props.headerActions}
        <IconButton title="Refresh" aria-label="Refresh" onClick={props.onRefresh}>
          <RefreshCw size={16} />
        </IconButton>
        <IconButton title="Import CLI sessions" aria-label="Import CLI sessions" onClick={props.onImportAll}>
          <DownloadCloud size={16} />
        </IconButton>
        <IconButton title="Add project" aria-label="Add project" onClick={props.onAddProject}>
          <FolderPlus size={16} />
        </IconButton>
      </header>
      <div class="companion-scroll">
        <AsyncContent
          state={state()}
          error={props.error()}
          onRetry={props.onRefresh}
          emptyTitle="No projects yet"
        >
          <div class="flex flex-col gap-2 p-3">
            <For each={props.projects()}>
              {(project) => (
                <section class="companion-project-card">
                  <button
                    type="button"
                    class="flex w-full items-center gap-2 px-3 py-2.5 text-left"
                    onClick={() => toggle(project.id)}
                    aria-expanded={isOpen(project.id)}
                  >
                    <ChevronRight
                      size={14}
                      class="theme-muted transition-transform"
                      classList={{ "rotate-90": isOpen(project.id) }}
                    />
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-sm font-semibold">{project.name}</span>
                      <span class="block truncate text-[11px] text-[var(--ui-text-caption)]">{project.rootPath}</span>
                    </span>
                    <Show when={byProject(project.id).length > 0}>
                      <Badge tone="neutral">{byProject(project.id).length}</Badge>
                    </Show>
                  </button>
                  <Show when={isOpen(project.id)}>
                    <div class="companion-project-sessions">
                      <Show
                        when={byProject(project.id).length > 0}
                        fallback={<p class="px-2 py-2 text-xs theme-muted">No open sessions</p>}
                      >
                        <For each={byProject(project.id)}>
                          {(s) => <SessionRow session={s} onOpen={props.onOpen} />}
                        </For>
                      </Show>
                      <div class="flex flex-wrap gap-2 px-1.5 pt-2 pb-1">
                        <Button variant="primary" size="sm" onClick={() => props.onNewSession(project.id)}>
                          <Plus size={13} /> New session
                        </Button>
                        <Button variant="outline" size="sm" onClick={() => props.onShowClosed(project)}>
                          <History size={13} /> History
                        </Button>
                        <Button variant="outline" size="sm" onClick={() => props.onImport(project)}>
                          <DownloadCloud size={13} /> Import CLI
                        </Button>
                      </div>
                    </div>
                  </Show>
                </section>
              )}
            </For>
            <Show when={unassigned().length > 0}>
              <section class="companion-project-card">
                <button
                  type="button"
                  class="flex w-full items-center gap-2 px-3 py-2.5 text-left"
                  onClick={() => toggle(UNASSIGNED)}
                >
                  <ChevronRight size={14} class="theme-muted transition-transform" classList={{ "rotate-90": open().has(UNASSIGNED) }} />
                  <span class="flex-1 text-sm font-semibold">No project</span>
                  <Badge tone="neutral">{unassigned().length}</Badge>
                </button>
                <Show when={open().has(UNASSIGNED)}>
                  <div class="companion-project-sessions">
                    <For each={unassigned()}>{(s) => <SessionRow session={s} onOpen={props.onOpen} />}</For>
                  </div>
                </Show>
              </section>
            </Show>
          </div>
        </AsyncContent>
      </div>
    </div>
  );
}
