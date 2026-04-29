import { For, Show, createMemo, createSignal } from "solid-js";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Folder,
  PanelRightClose,
  RefreshCw,
  Search,
} from "lucide-solid";
import type { GitStatusStore } from "../hooks/useGitPoller";
import type { GitFileEntry, GitState } from "../lib/tauriApi";
import {
  DropdownContent,
  DropdownItem,
  DropdownMenu,
  DropdownTrigger,
  EmptyState,
  ListRow,
  Panel,
  PanelBody,
  PanelHeader,
  PanelHeaderAction,
} from "./ui";
import FilesPanel from "./FilesPanel";
import FileGlyph from "./FileGlyph";

type WorkspaceFilesView = "changed" | "all";

type WorkspaceFilesPanelProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  gitStatus: () => GitStatusStore;
  onRefreshGit: () => void;
  onOpenFile: (relPath: string) => void;
  onOpenDiff: (path: string, staged: boolean, untracked: boolean) => void;
  onCollapse: () => void;
};

type ChangeEntry = GitFileEntry & { staged: boolean; untracked: boolean };

function statusView(gitStatus: GitStatusStore) {
  const s = gitStatus.state;
  return s && s.kind === "status" ? s : null;
}

function fileName(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? path : path.slice(i + 1);
}

function dirName(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? "" : path.slice(0, i);
}

function statusClass(entry: ChangeEntry): string {
  if (entry.untracked) return "git-status-untracked";
  switch (entry.statusLetter) {
    case "A": return "git-status-added";
    case "D": return "git-status-deleted";
    case "R":
    case "C": return "git-status-renamed";
    default: return "git-status-modified";
  }
}

function statusLetter(entry: ChangeEntry): string {
  if (entry.untracked) return "U";
  switch (entry.statusLetter) {
    case "A": return "A";
    case "D": return "D";
    case "R":
    case "C": return "R";
    default: return "M";
  }
}

export default function WorkspaceFilesPanel(props: WorkspaceFilesPanelProps) {
  const [view, setView] = createSignal<WorkspaceFilesView>("changed");
  const [viewMenuOpen, setViewMenuOpen] = createSignal(false);
  const [changedFilter, setChangedFilter] = createSignal("");
  const [openGroups, setOpenGroups] = createSignal<Record<string, boolean>>({});

  const git = () => statusView(props.gitStatus());
  const changedEntries = createMemo<ChangeEntry[]>(() => {
    const s = git();
    if (!s) return [];
    return [
      ...s.staged.map((entry) => ({ ...entry, staged: true, untracked: false })),
      ...s.unstaged.map((entry) => ({ ...entry, staged: false, untracked: false })),
      ...s.untracked.map((entry) => ({ ...entry, staged: false, untracked: true })),
    ];
  });
  const changedCount = () => changedEntries().length;
  const filteredChangedEntries = createMemo<ChangeEntry[]>(() => {
    const q = changedFilter().trim().toLowerCase();
    if (!q) return changedEntries();
    return changedEntries().filter((entry) => entry.path.toLowerCase().includes(q));
  });
  const groupedChanges = createMemo(() => {
    const groups: Array<{ dir: string; entries: ChangeEntry[] }> = [];
    const byDir = new Map<string, ChangeEntry[]>();
    for (const entry of filteredChangedEntries()) {
      const dir = dirName(entry.path);
      const list = byDir.get(dir);
      if (list) list.push(entry);
      else byDir.set(dir, [entry]);
    }
    for (const [dir, entries] of byDir) groups.push({ dir, entries });
    groups.sort((a, b) => a.dir.localeCompare(b.dir));
    return groups;
  });

  const title = () => view() === "changed" ? `Changed files ${changedCount()}` : "All files";
  const selectView = (next: WorkspaceFilesView) => {
    setView(next);
    setViewMenuOpen(false);
  };
  const groupIsOpen = (dir: string) => openGroups()[dir] !== false;
  const toggleGroup = (dir: string) => {
    setOpenGroups((prev) => ({ ...prev, [dir]: !groupIsOpen(dir) }));
  };

  const renderChanged = () => {
    const state = () => props.gitStatus().state;
    return (
      <>
        <Show when={state()?.kind === "git_missing"}>
          <EmptyState class="flex-1">git binary not found on PATH.</EmptyState>
        </Show>
        <Show when={state()?.kind === "not_repo"}>
          {() => {
            const s = state() as Extract<GitState, { kind: "not_repo" }>;
            return (
              <EmptyState class="flex-1">
                <div>Not a git repository</div>
                <div class="font-mono text-[10px] opacity-70 break-all">{s.cwd}</div>
              </EmptyState>
            );
          }}
        </Show>
        <Show when={!state() && props.gitStatus().loading}>
          <EmptyState class="flex-1">Loading...</EmptyState>
        </Show>
        <Show when={git()}>
          <PanelBody class="tool-panel-body flex-1 overflow-auto min-h-0">
            <label class="workspace-files-search">
              <Search size={13} />
              <input
                value={changedFilter()}
                onInput={(e) => setChangedFilter(e.currentTarget.value)}
                placeholder="Search changed files"
              />
            </label>
            <Show when={changedCount() === 0}>
              <EmptyState>Working tree clean</EmptyState>
            </Show>
            <Show when={changedCount() > 0 && filteredChangedEntries().length === 0}>
              <EmptyState>No matching files</EmptyState>
            </Show>
            <For each={groupedChanges()}>
              {(group) => (
                <div class="workspace-file-group">
                  <Show when={group.dir}>
                    <button
                      type="button"
                      class="workspace-file-group-label"
                      onClick={() => toggleGroup(group.dir)}
                    >
                      <ChevronRight
                        size={12}
                        class={`theme-muted transition-transform ${groupIsOpen(group.dir) ? "rotate-90" : ""}`}
                      />
                      <span class="truncate">{group.dir}</span>
                    </button>
                  </Show>
                  <Show when={!group.dir || groupIsOpen(group.dir)}>
                    <For each={group.entries}>
                      {(entry) => (
                        <ListRow
                          class="workspace-file-row git-change-row"
                          title={entry.path}
                          onClick={() => props.onOpenDiff(entry.path, entry.staged, entry.untracked)}
                        >
                          <FileGlyph name={fileName(entry.path)} />
                          <span class="min-w-0 flex-1 truncate">{fileName(entry.path)}</span>
                          <span class={`git-status ${statusClass(entry)}`}>{statusLetter(entry)}</span>
                        </ListRow>
                      )}
                    </For>
                  </Show>
                </div>
              )}
            </For>
          </PanelBody>
        </Show>
      </>
    );
  };

  return (
    <Panel class="tool-panel workspace-files-panel flex h-full flex-col overflow-hidden">
      <PanelHeader class="panel-header">
        <DropdownMenu open={viewMenuOpen()} onOpenChange={setViewMenuOpen}>
          <DropdownTrigger variant="plain" class="workspace-files-view-trigger" title="Choose file view">
            <Folder size={15} />
            <span>{title()}</span>
            <ChevronDown size={13} />
          </DropdownTrigger>
          <DropdownContent placement="bottom-start" class="workspace-files-view-menu">
            <DropdownItem onSelect={() => selectView("changed")}>
              <span>Changed files</span>
              <Show when={view() === "changed"}>
                <Check size={14} class="ml-auto theme-muted" />
              </Show>
            </DropdownItem>
            <DropdownItem onSelect={() => selectView("all")}>
              <span>All files</span>
              <Show when={view() === "all"}>
                <Check size={14} class="ml-auto theme-muted" />
              </Show>
            </DropdownItem>
          </DropdownContent>
        </DropdownMenu>
        <div class="ml-auto flex items-center gap-0.5">
          <PanelHeaderAction title="Refresh" onClick={() => props.onRefreshGit()}>
            <RefreshCw size={13} />
          </PanelHeaderAction>
          <PanelHeaderAction title="Hide files" onClick={() => props.onCollapse()}>
            <PanelRightClose size={13} />
          </PanelHeaderAction>
        </div>
      </PanelHeader>

      <Show when={view() === "changed"} fallback={
        <FilesPanel
          embedded
          appSessionId={props.appSessionId}
          cwd={props.cwd}
          onOpenFile={props.onOpenFile}
        />
      }>
        {renderChanged()}
      </Show>
    </Panel>
  );
}
