import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { gitApi, type GitFileEntry, type GitState } from "../lib/tauriApi";
import { INTERACTIVE_MOTION } from "./types";

type GitPanelProps = {
  appSessionId: () => string | undefined;
  onAddMention: (path: string) => void;
  onCollapse: () => void;
  onOpenDiff: (path: string, staged: boolean) => void;
};

const FOLDER_ROLLUP_THRESHOLD = 10;

type GroupRow =
  | { kind: "file"; entry: GitFileEntry }
  | { kind: "folder"; folder: string; count: number; entries: GitFileEntry[] };

function rollupGroup(entries: GitFileEntry[]): GroupRow[] {
  const byParent = new Map<string, GitFileEntry[]>();
  for (const e of entries) {
    const slash = e.path.lastIndexOf("/");
    const parent = slash === -1 ? "" : e.path.slice(0, slash);
    const arr = byParent.get(parent) ?? [];
    arr.push(e);
    byParent.set(parent, arr);
  }
  const rows: GroupRow[] = [];
  for (const [parent, items] of byParent) {
    if (parent && items.length > FOLDER_ROLLUP_THRESHOLD) {
      rows.push({ kind: "folder", folder: parent, count: items.length, entries: items });
    } else {
      for (const e of items) rows.push({ kind: "file", entry: e });
    }
  }
  return rows;
}

export default function GitPanel(props: GitPanelProps) {
  const sessionId = createMemo(() => props.appSessionId());
  const [statusRes, { refetch }] = createResource(sessionId, (sid) => gitApi.status(sid ?? null));

  const [stagedOpen, setStagedOpen] = createSignal(true);
  const [unstagedOpen, setUnstagedOpen] = createSignal(true);
  const [untrackedOpen, setUntrackedOpen] = createSignal(true);

  let unlisten: UnlistenFn | null = null;
  onMount(async () => {
    unlisten = await listen("git/changed", () => {
      refetch();
    });
  });
  onCleanup(() => {
    if (unlisten) unlisten();
  });

  const renderGroup = (
    title: string,
    open: () => boolean,
    setOpen: (v: boolean) => void,
    entries: GitFileEntry[],
    staged: boolean,
  ) => {
    const rows = createMemo(() => rollupGroup(entries));
    return (
      <div class="space-y-1">
        <button
          type="button"
          onClick={() => setOpen(!open())}
          class={`flex w-full items-center justify-between text-[10px] font-medium uppercase tracking-widest theme-muted ${INTERACTIVE_MOTION}`}
        >
          <span>{title} ({entries.length})</span>
          <svg
            class={`h-3 w-3 transition-transform ${open() ? "rotate-90" : ""}`}
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path d="M4 2l4 4-4 4" />
          </svg>
        </button>
        <Show when={open()}>
          <div class="space-y-0.5">
            <For each={rows()}>
              {(row) =>
                row.kind === "file" ? (
                  <div
                    class={`group flex items-center gap-2 rounded px-2 py-1 text-xs ${INTERACTIVE_MOTION} theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)]`}
                  >
                    <button
                      type="button"
                      onClick={() => props.onOpenDiff(row.entry.path, staged)}
                      class="flex-1 flex items-center gap-2 text-left truncate"
                    >
                      <span class="font-mono text-[10px] w-5 shrink-0 text-center opacity-70">
                        {row.entry.statusLetter}
                      </span>
                      <span class="truncate">{row.entry.path}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => props.onAddMention(row.entry.path)}
                      title="Add to chat as @mention"
                      class={`shrink-0 rounded px-1.5 py-0.5 text-[10px] opacity-0 group-hover:opacity-100 hover:bg-[var(--ui-panel)] ${INTERACTIVE_MOTION}`}
                    >
                      +@
                    </button>
                  </div>
                ) : (
                  <div class="group flex items-center gap-2 rounded px-2 py-1 text-xs theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)]">
                    <button
                      type="button"
                      onClick={() => props.onOpenDiff(`${row.folder}/`, staged)}
                      class="flex-1 flex items-center gap-2 text-left truncate"
                    >
                      <span class="font-mono text-[10px] w-5 shrink-0 text-center opacity-70">…</span>
                      <span class="truncate">{row.folder}/</span>
                      <span class="text-[10px] opacity-60">({row.count})</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => props.onAddMention(`${row.folder}/`)}
                      title="Add folder to chat as @mention"
                      class={`shrink-0 rounded px-1.5 py-0.5 text-[10px] opacity-0 group-hover:opacity-100 hover:bg-[var(--ui-panel)] ${INTERACTIVE_MOTION}`}
                    >
                      +@
                    </button>
                  </div>
                )
              }
            </For>
          </div>
        </Show>
      </div>
    );
  };

  return (
    <div class="flex flex-col h-full overflow-hidden theme-surface">
      <div class="flex items-center justify-between px-3 py-2 border-b theme-border shrink-0">
        <span class="text-[10px] font-medium uppercase tracking-widest theme-muted">Git</span>
        <div class="flex items-center gap-1">
          <button
            type="button"
            onClick={() => refetch()}
            title="Refresh"
            class={`flex h-6 w-6 items-center justify-center rounded theme-muted hover:text-primary ${INTERACTIVE_MOTION}`}
          >
            <svg viewBox="0 0 12 12" class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M2 6a4 4 0 1 1 1.2 2.85" />
              <path d="M2 9.5V6h3.5" />
            </svg>
          </button>
          <button
            type="button"
            onClick={() => props.onCollapse()}
            title="Collapse (Cmd/Ctrl+G)"
            class={`flex h-6 w-6 items-center justify-center rounded theme-muted hover:text-primary ${INTERACTIVE_MOTION}`}
          >
            <svg viewBox="0 0 12 12" class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M8 2L4 6l4 4" />
            </svg>
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
              <div class="flex-1 flex flex-col items-center justify-center p-6 text-center text-xs theme-muted">
                <div>Not a git repository</div>
                <div class="mt-2 font-mono text-[10px] opacity-70 break-all">{s.cwd}</div>
              </div>
            );
          }
          const dirty = s.staged.length + s.unstaged.length + s.untracked.length > 0;
          return (
            <>
              <div class="px-3 py-2 border-b theme-border shrink-0">
                <div class="flex items-center gap-2 text-xs">
                  <Show when={s.detached}>
                    <span class="rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-amber-300">
                      detached
                    </span>
                  </Show>
                  <span class="font-medium theme-text truncate">{s.branch ?? "(unknown)"}</span>
                </div>
                <Show when={s.upstream}>
                  <div class="mt-0.5 text-[10px] theme-muted truncate">→ {s.upstream}</div>
                </Show>
                <Show when={s.ahead > 0 || s.behind > 0}>
                  <div class="mt-1 text-[10px] theme-muted">
                    <Show when={s.ahead > 0}><span>↑{s.ahead}</span></Show>
                    <Show when={s.ahead > 0 && s.behind > 0}><span> </span></Show>
                    <Show when={s.behind > 0}><span>↓{s.behind}</span></Show>
                  </div>
                </Show>
              </div>

              <div class="flex-1 overflow-auto p-3 space-y-4 min-h-0">
                <Show when={!dirty}>
                  <div class="text-xs theme-muted text-center py-6">Working tree clean</div>
                </Show>
                <Show when={s.staged.length > 0}>
                  {renderGroup("Staged", stagedOpen, setStagedOpen, s.staged, true)}
                </Show>
                <Show when={s.unstaged.length > 0}>
                  {renderGroup("Unstaged", unstagedOpen, setUnstagedOpen, s.unstaged, false)}
                </Show>
                <Show when={s.untracked.length > 0}>
                  {renderGroup("Untracked", untrackedOpen, setUntrackedOpen, s.untracked, false)}
                </Show>
              </div>
            </>
          );
        }}
      </Show>
    </div>
  );
}
