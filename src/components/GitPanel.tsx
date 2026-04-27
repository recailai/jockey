import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import {
  ChevronRight,
  RefreshCw,
  PanelLeftClose,
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
  type GitFileEntry,
  type GitRemoteInfo,
  type GitState,
} from "../lib/tauriApi";
import type { GitStatusStore } from "../hooks/useGitPoller";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

type GitPanelProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  gitStatus: () => GitStatusStore;
  onRefresh: () => void;
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
  const [stagedOpen, setStagedOpen] = createSignal(true);
  const [changesOpen, setChangesOpen] = createSignal(true);

  const [branchMenuOpen, setBranchMenuOpen] = createSignal(false);
  const [branches, setBranches] = createSignal<BranchInfo[]>([]);
  const [branchFilter, setBranchFilter] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [checkoutError, setCheckoutError] = createSignal<string | null>(null);

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

  onMount(() => {
    void loadRemote();
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

  const isGithub = () => {
    const r = remoteInfo();
    return !!r && r.host.toLowerCase().includes("github.com");
  };

  return (
    <div class="flex flex-col h-full overflow-hidden theme-sidebar">
      <div class="panel-header">
        <span class="panel-header-title">Source Control</span>
        <div class="flex items-center gap-0.5">
          <button type="button" class="icon-btn" title="Refresh" onClick={() => props.onRefresh()}>
            <RefreshCw size={13} />
          </button>
          <button type="button" class="icon-btn" title="Collapse (Cmd/Ctrl+G)" onClick={() => props.onCollapse()}>
            <PanelLeftClose size={13} />
          </button>
        </div>
      </div>

      <Show when={state()?.kind === "git_missing"}>
        <div class="flex-1 flex items-center justify-center p-6 text-center text-xs theme-muted">
          git binary not found on PATH.
        </div>
      </Show>

      <Show when={state()?.kind === "not_repo"}>
        {(_s) => {
          const s = state() as Extract<GitState, { kind: "not_repo" }>;
          return (
            <div class="flex-1 flex flex-col items-center justify-center p-6 text-center text-xs theme-muted gap-2">
              <div>Not a git repository</div>
              <div class="font-mono text-[10px] opacity-70 break-all">{s.cwd}</div>
            </div>
          );
        }}
      </Show>

      <Show when={!state() && statusObj().loading}>
        <div class="flex-1 flex items-center justify-center text-xs theme-muted">Loading…</div>
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
              <div class="relative px-3 py-2 border-b theme-border">
                <button
                  type="button"
                  onClick={openBranchMenu}
                  onContextMenu={openContextMenu}
                  class="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md border theme-border theme-control hover:bg-[var(--ui-control-hover)] transition-colors group"
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
                    <span class="chip text-[9px]">detached</span>
                  </Show>
                  <ChevronDown size={12} class="shrink-0 theme-muted opacity-60 group-hover:opacity-100 transition-opacity" />
                </button>

                <Show when={branchMenuOpen()}>
                  <div
                    class="absolute left-3 right-3 top-[calc(100%-4px)] z-50 rounded-md border theme-border shadow-xl overflow-hidden"
                    style={{ "background-color": "var(--ui-bg)" }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <div class="px-2.5 py-1.5 border-b theme-border">
                      <input
                        type="text"
                        autofocus
                        placeholder="Filter branches…"
                        value={branchFilter()}
                        onInput={(e) => setBranchFilter(e.currentTarget.value)}
                        class="w-full px-2 py-1 text-[11px] rounded theme-input outline-none focus:border-[var(--ui-accent)]"
                      />
                    </div>
                    <Show when={checkoutError()}>
                      <div class="px-3 py-1.5 text-rose-400 text-[10px] border-b theme-border">
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
                            class="w-full flex items-center gap-2 px-3 py-1.5 text-left text-[11px] theme-text hover:bg-[var(--ui-accent-soft)] transition-colors disabled:opacity-50"
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

      <Show when={ctxMenu()}>
        {(pos) => {
          const r = remoteInfo();
          const branch = statusView()?.branch ?? null;
          const onGh = isGithub();
          return (
            <div
              class="fixed z-[200] min-w-[220px] overflow-hidden rounded-lg shadow-xl shadow-black/60 backdrop-blur-md py-1 theme-dropdown"
              style={`left:${pos().x}px;top:${pos().y}px`}
              onClick={(e) => e.stopPropagation()}
            >
              <Show when={r && branch}>
                <button
                  class="ctx-menu-item"
                  onClick={() => { void safeOpen(r?.branchUrl ?? r?.webUrl); setCtxMenu(null); }}
                >
                  <Globe size={12} />{onGh ? null : null}
                  <span>View branch on {onGh ? "GitHub" : r?.host}</span>
                </button>
                <button
                  class="ctx-menu-item"
                  onClick={() => { void safeOpen(r?.prUrl); setCtxMenu(null); }}
                >
                  <GitPullRequest size={12} />
                  <span>View pull request</span>
                </button>
                <button
                  class="ctx-menu-item"
                  onClick={() => { void safeOpen(r?.compareUrl); setCtxMenu(null); }}
                >
                  <ExternalLink size={12} />
                  <span>Open new pull request</span>
                </button>
                <div class="ctx-menu-sep" />
              </Show>
              <Show when={r && !branch}>
                <button
                  class="ctx-menu-item"
                  onClick={() => { void safeOpen(r?.webUrl); setCtxMenu(null); }}
                >
                  <Globe size={12} />{onGh ? null : null}
                  <span>View repository on {onGh ? "GitHub" : r?.host}</span>
                </button>
                <div class="ctx-menu-sep" />
              </Show>
              <Show when={branch}>
                <button
                  class="ctx-menu-item"
                  onClick={() => { void copyBranch(); setCtxMenu(null); }}
                >
                  <Copy size={12} />
                  <span>Copy branch name</span>
                </button>
              </Show>
              <button
                class="ctx-menu-item"
                onClick={() => { void revealInFinder(); setCtxMenu(null); }}
              >
                <FolderOpen size={12} />
                <span>Reveal in Finder</span>
              </button>
            </div>
          );
        }}
      </Show>
    </div>
  );
}
