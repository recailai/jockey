import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import {
  ChevronRight,
  RefreshCw,
  PanelRightClose,
  GitBranch,
  ChevronDown,
  Check,
  Globe,
  FolderOpen,
  Copy,
  ExternalLink,
  GitPullRequest,
} from "lucide-solid";
import {
  gitApi,
  type BranchInfo,
  type GitCommitEntry,
  type GitFileEntry,
  type GitRemoteInfo,
  type GitState,
} from "../lib/tauriApi";
import type { GitStatusStore } from "../hooks/useGitPoller";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { Badge, ContextMenuItem, ContextMenuSeparator, ContextMenuSurface, EmptyState, Input, ListRow, Panel, PanelBody, PanelHeader, PanelHeaderAction } from "./ui";
import FileGlyph from "./FileGlyph";

type GitPanelProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  gitStatus: () => GitStatusStore;
  onRefresh: () => void;
  onAddMention: (path: string) => void;
  onCollapse: () => void;
  onOpenDiff: (path: string, staged: boolean, untracked: boolean) => void;
  onOpenCommitDiff: (oid: string, label: string) => void;
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

function fmtRelative(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  if (diff < 604_800_000) return `${Math.floor(diff / 86_400_000)}d ago`;
  return new Date(ts).toLocaleDateString();
}

export default function GitPanel(props: GitPanelProps) {
  const [stagedOpen, setStagedOpen] = createSignal(true);
  const [changesOpen, setChangesOpen] = createSignal(true);
  const [historyOpen, setHistoryOpen] = createSignal(true);

  const [branchMenuOpen, setBranchMenuOpen] = createSignal(false);
  const [branches, setBranches] = createSignal<BranchInfo[]>([]);
  const [branchFilter, setBranchFilter] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [checkoutError, setCheckoutError] = createSignal<string | null>(null);
  const [history, setHistory] = createSignal<GitCommitEntry[]>([]);
  const [historyLoading, setHistoryLoading] = createSignal(false);
  const [historyError, setHistoryError] = createSignal<string | null>(null);

  const [ctxMenu, setCtxMenu] = createSignal<{ x: number; y: number } | null>(null);
  const [remoteInfo, setRemoteInfo] = createSignal<GitRemoteInfo | null>(null);

  const statusObj = () => props.gitStatus();
  const state = () => statusObj().state;
  const statusView = () => {
    const s = state();
    return s && s.kind === "status" ? s : null;
  };

  const loadRemote = async () => {
    try {
      const info = await gitApi.remoteInfo(props.appSessionId() ?? null);
      setRemoteInfo(info);
    } catch {
      setRemoteInfo(null);
    }
  };

  const loadHistory = async () => {
    if (!props.appSessionId()) {
      setHistory([]);
      return;
    }
    setHistoryLoading(true);
    setHistoryError(null);
    try {
      const entries = await gitApi.log(props.appSessionId() ?? null, 30);
      setHistory(entries);
    } catch (err) {
      setHistory([]);
      setHistoryError(String(err));
    } finally {
      setHistoryLoading(false);
    }
  };

  onMount(() => {
    void loadRemote();
    void loadHistory();
    const closeOnGlobal = () => {
      setCtxMenu(null);
      setBranchMenuOpen(false);
    };
    window.addEventListener("click", closeOnGlobal);
    window.addEventListener("blur", closeOnGlobal);
    onCleanup(() => {
      window.removeEventListener("click", closeOnGlobal);
      window.removeEventListener("blur", closeOnGlobal);
    });
  });

  createEffect(() => {
    statusView()?.branch;
    void loadHistory();
  });

  const openBranchMenu = async (e: MouseEvent) => {
    e.stopPropagation();
    setCheckoutError(null);
    setBranchFilter("");
    setBranchMenuOpen(true);
    try {
      const list = await gitApi.listBranches(props.appSessionId() ?? null);
      setBranches(list);
    } catch {
      setBranches([]);
    }
  };

  const switchBranch = async (branch: string) => {
    if (checking()) return;
    setChecking(true);
    setCheckoutError(null);
    try {
      await gitApi.checkout(props.appSessionId() ?? null, branch);
      setBranchMenuOpen(false);
      props.onRefresh();
      void loadRemote();
      void loadHistory();
    } catch (e) {
      setCheckoutError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const openContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    void loadRemote();
    setCtxMenu({ x: e.clientX, y: e.clientY });
  };

  const filteredBranches = () => {
    const q = branchFilter().trim().toLowerCase();
    if (!q) return branches();
    return branches().filter((b) => b.name.toLowerCase().includes(q));
  };

  const safeOpen = async (url: string | null | undefined) => {
    if (!url) {
      console.warn("[git-panel] openUrl: empty url");
      return;
    }
    try {
      await openUrl(url);
    } catch (err) {
      console.error("[git-panel] openUrl failed", url, err);
    }
  };

  const copyBranch = async () => {
    const name = statusView()?.branch;
    if (!name) return;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(name);
      }
    } catch {}
  };

  const revealInFinder = async () => {
    const cwd = props.cwd();
    if (!cwd) return;
    try { await revealItemInDir(cwd); } catch {}
  };

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
                <ListRow
                  class="git-change-row group"
                  onClick={() => props.onOpenDiff(entry.path, staged, entry.untracked)}
                  title={entry.path}
                >
                  <FileGlyph name={basename()} />
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
                </ListRow>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );

  const isGithub = () => {
    const r = remoteInfo();
    return !!r && r.host.toLowerCase().includes("github.com");
  };

  return (
    <Panel class="tool-panel flex h-full flex-col overflow-hidden">
      <PanelHeader class="panel-header">
        <span class="panel-header-title">Source Control</span>
        <div class="flex items-center gap-0.5">
          <PanelHeaderAction title="Refresh" onClick={() => { props.onRefresh(); void loadRemote(); void loadHistory(); }}>
            <RefreshCw size={13} />
          </PanelHeaderAction>
          <PanelHeaderAction title="Collapse (Cmd/Ctrl+G)" onClick={() => props.onCollapse()}>
            <PanelRightClose size={13} />
          </PanelHeaderAction>
        </div>
      </PanelHeader>

      <Show when={state()?.kind === "git_missing"}>
        <EmptyState class="flex-1">
          git binary not found on PATH.
        </EmptyState>
      </Show>

      <Show when={state()?.kind === "not_repo"}>
        {(_s) => {
          const s = state() as Extract<GitState, { kind: "not_repo" }>;
          return (
            <EmptyState class="flex-1">
              <div>Not a git repository</div>
              <div class="font-mono text-[10px] opacity-70 break-all">{s.cwd}</div>
            </EmptyState>
          );
        }}
      </Show>

      <Show when={!state() && statusObj().loading}>
        <EmptyState class="flex-1">Loading…</EmptyState>
      </Show>

      <Show when={statusView()}>
        {(_) => {
          const s = statusView()!;
          const stagedEntries = (): ChangeEntry[] =>
            s.staged.map((e) => ({ ...e, untracked: false }));
          const changesEntries = (): ChangeEntry[] => [
            ...s.unstaged.map((e) => ({ ...e, untracked: false })),
            ...s.untracked.map((e) => ({ ...e, untracked: true })),
          ];
          const dirty = stagedEntries().length + changesEntries().length > 0;

          return (
            <>
              <div class="tool-panel-section relative">
                <button
                  type="button"
                  onClick={openBranchMenu}
                  onContextMenu={openContextMenu}
                  class="tool-panel-card w-full flex items-center gap-2 group"
                  title="Left-click: switch branch · Right-click: more actions"
                >
                  <GitBranch size={13} class="shrink-0 theme-muted" />
                  <div class="flex flex-col items-start min-w-0 flex-1">
                    <span class="text-[10px] theme-muted uppercase tracking-wider leading-none">
                      Current branch
                    </span>
                    <span class="text-[12px] theme-text font-medium truncate max-w-full mt-0.5">
                      {s.branch ?? "(detached)"}
                    </span>
                  </div>
                  <Show when={s.ahead > 0 || s.behind > 0}>
                    <div class="flex items-center gap-1 text-[10.5px] theme-muted shrink-0">
                      <Show when={s.behind > 0}><span>↓{s.behind}</span></Show>
                      <Show when={s.ahead > 0}><span>↑{s.ahead}</span></Show>
                    </div>
                  </Show>
                  <Show when={s.detached}>
                    <Badge class="git-detached-badge">detached</Badge>
                  </Show>
                  <ChevronDown size={12} class="shrink-0 theme-muted opacity-60 group-hover:opacity-100 transition-opacity" />
                </button>

                <Show when={branchMenuOpen()}>
                  <div
                    class="git-branch-menu"
                    style={{ "background-color": "var(--ui-bg)" }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <div class="px-2.5 py-1.5 border-b theme-border">
                      <Input
                        type="text"
                        autofocus
                        placeholder="Filter branches…"
                        value={branchFilter()}
                        onInput={(e) => setBranchFilter(e.currentTarget.value)}
                        class="git-branch-filter"
                      />
                    </div>
                    <Show when={checkoutError()}>
                      <div class="px-3 py-1.5 text-[var(--ui-state-danger-text)] text-[10px] border-b theme-border">
                        {checkoutError()}
                      </div>
                    </Show>
                    <div class="overflow-y-auto" style={{ "max-height": "260px" }}>
                      <For each={filteredBranches()} fallback={
                        <div class="px-3 py-2 theme-muted text-[11px]">No branches</div>
                      }>
                        {(b) => (
                          <button
                            type="button"
                            disabled={checking()}
                            onClick={() => !b.isCurrent && void switchBranch(b.name)}
                            class="git-branch-row"
                            classList={{ "cursor-default": b.isCurrent }}
                          >
                            <Show when={b.isCurrent} fallback={<span class="w-3 shrink-0" />}>
                              <Check size={12} class="shrink-0 text-[var(--ui-accent)]" />
                            </Show>
                            <span class="truncate flex-1">{b.name}</span>
                            <Show when={b.upstream}>
                              <span class="theme-muted text-[10px] truncate max-w-[100px]">{b.upstream}</span>
                            </Show>
                          </button>
                        )}
                      </For>
                    </div>
                    <Show when={checking()}>
                      <div class="border-t theme-border px-3 py-1 flex items-center gap-1.5">
                        <RefreshCw size={10} class="animate-spin theme-muted" />
                        <span class="theme-muted text-[10px]">Switching…</span>
                      </div>
                    </Show>
                  </div>
                </Show>
              </div>

              <PanelBody class="tool-panel-body flex-1 overflow-auto min-h-0">
                <Show when={!dirty}>
                  <EmptyState>
                    Working tree clean
                  </EmptyState>
                </Show>
                <Show when={stagedEntries().length > 0}>
                  {renderGroup("Staged Changes", stagedOpen, setStagedOpen, stagedEntries(), true)}
                </Show>
                <Show when={changesEntries().length > 0}>
                  {renderGroup("Changes", changesOpen, setChangesOpen, changesEntries(), false)}
                </Show>

                <div class="mt-3">
                  <button
                    type="button"
                    onClick={() => setHistoryOpen(!historyOpen())}
                    class="section-header"
                  >
                    <ChevronRight
                      size={12}
                      class={`transition-transform shrink-0 ${historyOpen() ? "rotate-90" : ""}`}
                    />
                    <span>Recent Commits</span>
                    <span class="section-count">{history().length}</span>
                  </button>
                  <Show when={historyOpen()}>
                    <div class="mt-1">
                      <Show when={historyLoading()}>
                        <div class="px-2 py-2 text-[11px] theme-muted">Loading history…</div>
                      </Show>
                      <Show when={historyError()}>
                        {(error) => <div class="px-2 py-2 text-[11px] text-[var(--ui-state-danger-text)]">{error()}</div>}
                      </Show>
                      <For each={history()}>
                        {(entry) => (
                          <ListRow
                            class="git-commit-row"
                            onClick={() => props.onOpenCommitDiff(entry.oid, `${entry.shortOid} ${entry.summary}`)}
                            title={entry.oid}
                          >
                            <div class="min-w-0 flex-1">
                              <div class="flex items-center gap-2 min-w-0">
                                <span class="font-mono text-[10.5px] theme-muted shrink-0">{entry.shortOid}</span>
                                <span class="truncate theme-text">{entry.summary}</span>
                              </div>
                              <div class="mt-0.5 flex items-center gap-2 text-[10.5px] theme-muted">
                                <span class="truncate">{entry.authorName}</span>
                                <span>{fmtRelative(entry.committedAt)}</span>
                              </div>
                            </div>
                            <ExternalLink size={12} class="shrink-0 theme-muted" />
                          </ListRow>
                        )}
                      </For>
                      <Show when={!historyLoading() && !historyError() && history().length === 0}>
                        <div class="px-2 py-2 text-[11px] theme-muted">No commits on this branch yet.</div>
                      </Show>
                    </div>
                  </Show>
                </div>
              </PanelBody>
            </>
          );
        }}
      </Show>

      <Show when={ctxMenu()}>
        {(pos) => {
          const r = remoteInfo();
          const branch = statusView()?.branch ?? null;
          const onGh = isGithub();
          return (
            <ContextMenuSurface
              x={pos().x}
              y={pos().y}
              width={220}
              onClick={(e) => e.stopPropagation()}
            >
              <Show when={r && branch}>
                <ContextMenuItem
                  icon={<Globe size={12} />}
                  onSelect={() => { void safeOpen(r?.branchUrl ?? r?.webUrl); setCtxMenu(null); }}
                >
                  <span>View branch on {onGh ? "GitHub" : r?.host}</span>
                </ContextMenuItem>
                <ContextMenuItem
                  icon={<GitPullRequest size={12} />}
                  onSelect={() => { void safeOpen(r?.prUrl); setCtxMenu(null); }}
                >
                  <span>View pull request</span>
                </ContextMenuItem>
                <ContextMenuItem
                  icon={<ExternalLink size={12} />}
                  onSelect={() => { void safeOpen(r?.compareUrl); setCtxMenu(null); }}
                >
                  <span>Open new pull request</span>
                </ContextMenuItem>
                <ContextMenuSeparator />
              </Show>
              <Show when={r && !branch}>
                <ContextMenuItem
                  icon={<Globe size={12} />}
                  onSelect={() => { void safeOpen(r?.webUrl); setCtxMenu(null); }}
                >
                  <span>View repository on {onGh ? "GitHub" : r?.host}</span>
                </ContextMenuItem>
                <ContextMenuSeparator />
              </Show>
              <Show when={branch}>
                <ContextMenuItem
                  icon={<Copy size={12} />}
                  onSelect={() => { void copyBranch(); setCtxMenu(null); }}
                >
                  <span>Copy branch name</span>
                </ContextMenuItem>
              </Show>
              <ContextMenuItem
                icon={<FolderOpen size={12} />}
                onSelect={() => { void revealInFinder(); setCtxMenu(null); }}
              >
                <span>Reveal in Finder</span>
              </ContextMenuItem>
            </ContextMenuSurface>
          );
        }}
      </Show>
    </Panel>
  );
}
