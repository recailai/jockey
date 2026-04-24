import { For, Show, createResource, createSignal } from "solid-js";
import { ChevronRight, RefreshCw, PanelLeftClose, GitBranch } from "lucide-solid";
import { gitApi, type GitFileEntry, type GitState } from "../lib/tauriApi";
import { useGitChanged } from "../hooks/useGitChanged";

type GitPanelProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  onAddMention: (path: string) => void;
  onCollapse: () => void;
  onOpenDiff: (path: string, staged: boolean, untracked: boolean) => void;
};

type ChangeEntry = GitFileEntry & { untracked: boolean };

function vscodeLetter(entry: ChangeEntry): string {
  if (entry.untracked) return "U";
  switch (entry.statusLetter) {
    case "A": return "A";
    case "D": return "D";
    case "R":
    case "C": return "R";
    default:   return "M";
  }
}

function statusClass(entry: ChangeEntry): string {
  if (entry.untracked) return "git-status-untracked";
  switch (entry.statusLetter) {
    case "A": return "git-status-added";
    case "D": return "git-status-deleted";
    case "R":
    case "C": return "git-status-renamed";
    default:   return "git-status-modified";
  }
}

export default function GitPanel(props: GitPanelProps) {
  const [statusRes, { refetch }] = createResource(
    () => props.appSessionId() ?? null,
    (sid) => gitApi.status(sid),
  );

  const [stagedOpen, setStagedOpen] = createSignal(true);
  const [changesOpen, setChangesOpen] = createSignal(true);

  useGitChanged(props.cwd, () => { void refetch(); });

  const renderGroup = (
    title: string,
    open: () => boolean,
    setOpen: (v: boolean) => void,
    entries: ChangeEntry[],
    staged: boolean,
  ) => (
    <div class="mb-1">
      <button
        type="button"
        onClick={() => setOpen(!open())}
        class="section-header"
      >
        <ChevronRight
          size={12}
          class={`transition-transform shrink-0 ${open() ? "rotate-90" : ""}`}
        />
        <span>{title}</span>
        <span class="section-count">{entries.length}</span>
      </button>
      <Show when={open()}>
        <div>
          <For each={entries}>
            {(entry) => {
              const basename = () => {
                const i = entry.path.lastIndexOf("/");
                return i === -1 ? entry.path : entry.path.slice(i + 1);
              };
              const dirname = () => {
                const i = entry.path.lastIndexOf("/");
                return i === -1 ? "" : entry.path.slice(0, i);
              };
              return (
                <div
                  class="row-item group"
                  onClick={() => props.onOpenDiff(entry.path, staged, entry.untracked)}
                  title={entry.path}
                >
                  <span class="truncate">{basename()}</span>
                  <Show when={dirname()}>
                    <span class="text-[10.5px] theme-muted truncate min-w-0 flex-1">{dirname()}</span>
                  </Show>
                  <button
                    type="button"
                    onClick={(e) => { e.stopPropagation(); props.onAddMention(entry.path); }}
                    title="Add to chat as @mention"
                    class="shrink-0 rounded px-1.5 py-0.5 text-[10px] theme-muted hover:theme-text hover:bg-[var(--ui-accent-soft)] opacity-0 group-hover:opacity-100 transition-opacity"
                  >
                    +@
                  </button>
                  <span class={`git-status ${statusClass(entry)}`}>
                    {vscodeLetter(entry)}
                  </span>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );

  return (
    <div class="flex flex-col h-full overflow-hidden theme-bg">
      <div class="panel-header">
        <span class="panel-header-title">Source Control</span>
        <div class="flex items-center gap-0.5">
          <button type="button" class="icon-btn" title="Refresh" onClick={() => refetch()}>
            <RefreshCw size={13} />
          </button>
          <button type="button" class="icon-btn" title="Collapse (Cmd/Ctrl+G)" onClick={() => props.onCollapse()}>
            <PanelLeftClose size={13} />
          </button>
        </div>
      </div>

      <Show when={statusRes.loading}>
        <div class="flex-1 flex items-center justify-center text-xs theme-muted">Loading…</div>
      </Show>

      <Show when={!statusRes.loading && statusRes()}>
        {(state) => {
          const s = state() as GitState;
          if (s.kind === "git_missing") {
            return (
              <div class="flex-1 flex items-center justify-center p-6 text-center text-xs theme-muted">
                git binary not found on PATH.
              </div>
            );
          }
          if (s.kind === "not_repo") {
            return (
              <div class="flex-1 flex flex-col items-center justify-center p-6 text-center text-xs theme-muted gap-2">
                <div>Not a git repository</div>
                <div class="font-mono text-[10px] opacity-70 break-all">{s.cwd}</div>
              </div>
            );
          }

          const stagedEntries = (): ChangeEntry[] =>
            s.staged.map((e) => ({ ...e, untracked: false }));

          const changesEntries = (): ChangeEntry[] => [
            ...s.unstaged.map((e) => ({ ...e, untracked: false })),
            ...s.untracked.map((e) => ({ ...e, untracked: true })),
          ];

          const dirty = stagedEntries().length + changesEntries().length > 0;

          return (
            <>
              <div class="panel-subheader">
                <GitBranch size={12} class="shrink-0 theme-muted" />
                <span class="theme-text truncate font-medium">{s.branch ?? "(unknown)"}</span>
                <Show when={s.detached}>
                  <span class="chip text-[9px]">detached</span>
                </Show>
                <Show when={s.ahead > 0 || s.behind > 0}>
                  <span class="ml-auto flex items-center gap-1.5 text-[10.5px] theme-muted">
                    <Show when={s.behind > 0}><span>↓{s.behind}</span></Show>
                    <Show when={s.ahead > 0}><span>↑{s.ahead}</span></Show>
                  </span>
                </Show>
              </div>

              <div class="flex-1 overflow-auto py-2 min-h-0">
                <Show when={!dirty}>
                  <div class="text-xs theme-muted text-center py-10 px-6 leading-relaxed">
                    Working tree clean
                  </div>
                </Show>
                <Show when={stagedEntries().length > 0}>
                  {renderGroup("Staged Changes", stagedOpen, setStagedOpen, stagedEntries(), true)}
                </Show>
                <Show when={changesEntries().length > 0}>
                  {renderGroup("Changes", changesOpen, setChangesOpen, changesEntries(), false)}
                </Show>
              </div>
            </>
          );
        }}
      </Show>
    </div>
  );
}
