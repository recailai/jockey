import { Show, createResource, createSignal, onCleanup, onMount, For } from "solid-js";
import { GitBranch, AlertTriangle, GitPullRequest, Check, RefreshCw, GitCommit } from "lucide-solid";
import { gitApi, type GitState, type BranchInfo } from "../lib/tauriApi";
import { useGitChanged } from "../hooks/useGitChanged";
import { openUrl } from "@tauri-apps/plugin-opener";

type StatusBarProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  onOpenGit: () => void;
};

export default function StatusBar(props: StatusBarProps) {
  const [statusRes, { refetch: refetchStatus }] = createResource(
    () => props.appSessionId() ?? null,
    (sid) => gitApi.status(sid),
  );

  const [branchMenuOpen, setBranchMenuOpen] = createSignal(false);
  const [branches, setBranches] = createSignal<BranchInfo[]>([]);
  const [checkoutError, setCheckoutError] = createSignal<string | null>(null);
  const [checking, setChecking] = createSignal(false);

  useGitChanged(props.cwd, () => { void refetchStatus(); });

  let pollTimer: ReturnType<typeof setInterval> | null = null;
  onMount(() => {
    pollTimer = setInterval(() => { void refetchStatus(); }, 5000);
  });
  onCleanup(() => {
    if (pollTimer !== null) clearInterval(pollTimer);
  });

  const state = () => statusRes() as GitState | undefined;
  const statusView = () => {
    const s = state();
    return s && s.kind === "status" ? s : null;
  };

  const dirtyCount = () => {
    const s = statusView();
    if (!s) return 0;
    return s.staged.length + s.unstaged.length + s.untracked.length;
  };

  const openBranchMenu = async () => {
    setCheckoutError(null);
    setBranchMenuOpen(true);
    const sid = props.appSessionId() ?? null;
    try {
      const list = await gitApi.listBranches(sid);
      setBranches(list);
    } catch {
      setBranches([]);
    }
  };

  const closeBranchMenu = () => {
    setBranchMenuOpen(false);
    setCheckoutError(null);
  };

  const switchBranch = async (branch: string) => {
    if (checking()) return;
    setChecking(true);
    setCheckoutError(null);
    try {
      await gitApi.checkout(props.appSessionId() ?? null, branch);
      closeBranchMenu();
      void refetchStatus();
    } catch (e) {
      setCheckoutError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const openPr = async () => {
    const s = statusView();
    if (!s?.branch) return;
    try {
      const url = await gitApi.prUrl(props.appSessionId() ?? null);
      if (url) {
        await openUrl(url);
      }
    } catch {
    }
  };

  return (
    <div
      class="flex items-stretch shrink-0 border-t theme-border text-[11px] select-none theme-bg relative"
      style={{ height: "22px" }}
    >
      <Show when={statusView() !== null}>
        <button
          type="button"
          onClick={openBranchMenu}
          title="Switch branch"
          class="flex items-center gap-1.5 px-2.5 hover:bg-[var(--ui-accent-soft)] transition-colors theme-text"
        >
          <GitBranch size={12} class="shrink-0 theme-muted" />
          <span class="truncate max-w-[240px]">{statusView()?.branch ?? "(detached)"}</span>
          <Show when={(statusView()?.ahead ?? 0) > 0 || (statusView()?.behind ?? 0) > 0}>
            <span class="flex items-center gap-1 theme-muted">
              <Show when={(statusView()?.behind ?? 0) > 0}><span>↓{statusView()?.behind}</span></Show>
              <Show when={(statusView()?.ahead ?? 0) > 0}><span>↑{statusView()?.ahead}</span></Show>
            </span>
          </Show>
          <Show when={dirtyCount() > 0}>
            <span class="theme-muted">●{dirtyCount()}</span>
          </Show>
        </button>

        <button
          type="button"
          onClick={() => props.onOpenGit()}
          title="Open Source Control"
          class="flex items-center gap-1 px-2 hover:bg-[var(--ui-accent-soft)] transition-colors theme-muted"
        >
          <GitCommit size={11} />
        </button>

        <Show when={statusView()?.branch}>
          <button
            type="button"
            onClick={() => void openPr()}
            title="View Pull Request"
            class="flex items-center gap-1 px-2 hover:bg-[var(--ui-accent-soft)] transition-colors theme-muted"
          >
            <GitPullRequest size={11} />
          </button>
        </Show>

        <Show when={branchMenuOpen()}>
          <div
            class="fixed inset-0 z-40"
            onClick={closeBranchMenu}
          />
          <div
            class="absolute bottom-full left-0 z-50 mb-1 min-w-[220px] max-w-[320px] rounded border theme-border shadow-lg overflow-hidden"
            style={{ "background-color": "var(--ui-bg)" }}
          >
            <div class="px-3 py-1.5 text-[10px] theme-muted border-b theme-border font-medium uppercase tracking-wider">
              Switch Branch
            </div>
            <Show when={checkoutError()}>
              <div class="px-3 py-1.5 text-rose-400 text-[10px] border-b theme-border">
                {checkoutError()}
              </div>
            </Show>
            <div class="overflow-y-auto" style={{ "max-height": "240px" }}>
              <For each={branches()} fallback={
                <div class="px-3 py-2 theme-muted text-[11px]">Loading…</div>
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
                      <span class="theme-muted text-[10px] truncate max-w-[80px]">{b.upstream}</span>
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
      </Show>

      <Show when={state()?.kind === "not_repo"}>
        <div class="flex items-center gap-1.5 px-2.5 theme-muted" title="Not a git repository">
          <AlertTriangle size={12} />
          <span>not a repo</span>
        </div>
      </Show>
      <Show when={state()?.kind === "git_missing"}>
        <div class="flex items-center gap-1.5 px-2.5 text-rose-400" title="git binary not found">
          <AlertTriangle size={12} />
          <span>git missing</span>
        </div>
      </Show>
      <div class="flex-1" />
    </div>
  );
}
