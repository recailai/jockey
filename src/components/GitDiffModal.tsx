import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { gitApi } from "../lib/tauriApi";
import { INTERACTIVE_MOTION } from "./types";

type GitDiffModalProps = {
  appSessionId: () => string | undefined;
  path: string;
  staged: boolean;
  onClose: () => void;
  onAddMention: (path: string) => void;
};

export default function GitDiffModal(props: GitDiffModalProps) {
  const [vsHead, setVsHead] = createSignal(false);

  const query = createMemo(() => ({
    sid: props.appSessionId(),
    path: props.path,
    staged: props.staged,
    vsHead: vsHead(),
  }));

  const [diffRes] = createResource(query, async (q) =>
    gitApi.diff(q.sid, q.path, q.vsHead, q.staged),
  );

  const handleKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose();
    }
  };
  onMount(() => window.addEventListener("keydown", handleKey));
  onCleanup(() => window.removeEventListener("keydown", handleKey));

  return (
    <div
      class="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 backdrop-blur-sm p-6"
      onClick={() => props.onClose()}
    >
      <div
        class="theme-surface theme-border border rounded-lg shadow-2xl w-full max-w-5xl h-full max-h-[85vh] flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div class="flex items-center justify-between px-4 py-2.5 border-b theme-border shrink-0 gap-3">
          <div class="flex items-center gap-2 min-w-0 flex-1">
            <span class="text-[10px] font-medium uppercase tracking-widest theme-muted shrink-0">Diff</span>
            <span class="font-mono text-xs theme-text truncate">{props.path}</span>
            <Show when={props.staged}>
              <span class="shrink-0 rounded bg-emerald-500/20 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-emerald-300">
                staged
              </span>
            </Show>
          </div>
          <div class="flex items-center gap-3 shrink-0">
            <label class="flex items-center gap-1.5 text-[11px] theme-muted cursor-pointer">
              <input
                type="checkbox"
                checked={vsHead()}
                onChange={(e) => setVsHead(e.currentTarget.checked)}
              />
              vs HEAD
            </label>
            <button
              type="button"
              onClick={() => props.onAddMention(props.path)}
              title="Add to chat as @mention"
              class={`rounded px-2 py-1 text-[11px] theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)] ${INTERACTIVE_MOTION}`}
            >
              +@ mention
            </button>
            <button
              type="button"
              onClick={() => props.onClose()}
              title="Close (Esc)"
              class={`flex h-6 w-6 items-center justify-center rounded theme-muted hover:text-primary ${INTERACTIVE_MOTION}`}
            >
              <svg viewBox="0 0 12 12" class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M1 1l10 10M11 1L1 11" />
              </svg>
            </button>
          </div>
        </div>

        <div class="flex-1 overflow-auto px-4 py-3 min-h-0">
          <Show when={diffRes.loading}>
            <div class="text-xs theme-muted">Loading diff…</div>
          </Show>
          <Show when={!diffRes.loading && diffRes()}>
            <pre class="font-mono text-[12px] leading-snug whitespace-pre theme-text">
              <For each={(diffRes() ?? "").split("\n")}>
                {(line) => (
                  <div
                    class={
                      line.startsWith("+") && !line.startsWith("+++")
                        ? "text-emerald-400"
                        : line.startsWith("-") && !line.startsWith("---")
                        ? "text-rose-400"
                        : line.startsWith("@@")
                        ? "text-sky-400"
                        : ""
                    }
                  >
                    {line || "\u00A0"}
                  </div>
                )}
              </For>
            </pre>
          </Show>
          <Show when={!diffRes.loading && !diffRes()}>
            <div class="text-xs theme-muted">No changes</div>
          </Show>
        </div>
      </div>
    </div>
  );
}
