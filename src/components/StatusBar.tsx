import { Show } from "solid-js";
import { GitBranch, AlertTriangle } from "lucide-solid";
import { type GitState } from "../lib/tauriApi";
import type { GitStatusStore } from "../hooks/useGitPoller";

type StatusBarProps = {
  appSessionId: () => string | undefined;
  cwd: () => string | null;
  gitStatus: () => GitStatusStore;
  onOpenGit: () => void;
};

export default function StatusBar(props: StatusBarProps) {
  const state = () => props.gitStatus().state as GitState | null;
  const statusView = () => {
    const s = state();
    return s && s.kind === "status" ? s : null;
  };

  const dirtyCount = () => {
    const s = statusView();
    if (!s) return 0;
    return s.staged.length + s.unstaged.length + s.untracked.length;
  };

  return (
    <div
      class="flex items-stretch shrink-0 border-t theme-border text-[11px] select-none theme-bg"
      style={{ height: "22px" }}
    >
      <Show when={statusView()}>
        {(_) => {
          const s = statusView()!;
          return (
            <button
              type="button"
              onClick={() => props.onOpenGit()}
              title="Open Source Control"
              class="flex items-center gap-1.5 px-2.5 hover:bg-[var(--ui-accent-soft)] transition-colors theme-text"
            >
              <GitBranch size={12} class="shrink-0 theme-muted" />
              <span class="truncate max-w-[240px]">{s.branch ?? "(detached)"}</span>
              <Show when={s.ahead > 0 || s.behind > 0}>
                <span class="flex items-center gap-1 theme-muted">
                  <Show when={s.behind > 0}><span>↓{s.behind}</span></Show>
                  <Show when={s.ahead > 0}><span>↑{s.ahead}</span></Show>
                </span>
              </Show>
              <Show when={dirtyCount() > 0}>
                <span class="theme-muted">●{dirtyCount()}</span>
              </Show>
            </button>
          );
        }}
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
