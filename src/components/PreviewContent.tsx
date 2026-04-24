import { Show, createMemo, createResource, createSignal } from "solid-js";
import { AtSign } from "lucide-solid";
import { gitApi } from "../lib/tauriApi";
import { renderMd } from "../lib/markdown";
import type { PreviewMode } from "./types";
import DiffView from "./DiffView";

type PreviewContentProps = {
  appSessionId: () => string | undefined;
  path: string;
  initialMode: PreviewMode;
  staged: boolean;
  untracked: boolean;
  version: () => number;
  onAddMention?: (path: string) => void;
};

const MD_EXT = /\.(md|markdown|mdx)$/i;
const isMarkdown = (p: string) => MD_EXT.test(p);

export default function PreviewContent(props: PreviewContentProps) {
  const isFolder = () => props.path.endsWith("/");
  const pathIsMd = createMemo(() => isMarkdown(props.path));

  const computeInitial = (): PreviewMode => {
    if (props.initialMode === "diff") return "diff";
    return pathIsMd() ? "preview" : "file";
  };
  const [mode, setMode] = createSignal<PreviewMode>(computeInitial());
  const [vsHead, setVsHead] = createSignal(false);

  const diffQuery = createMemo(() => ({
    sid: props.appSessionId(),
    path: props.path,
    staged: props.staged,
    vsHead: vsHead(),
    v: props.version(),
  }));

  const [diffRes] = createResource(
    () => (mode() === "diff" ? diffQuery() : null),
    async (q) => (q ? gitApi.diff(q.sid, q.path, q.vsHead, q.staged, props.untracked) : ""),
  );

  const fileQuery = createMemo(() => ({
    sid: props.appSessionId(),
    path: props.path,
    v: props.version(),
  }));

  const [fileRes] = createResource(
    () => (mode() === "file" || mode() === "preview" ? fileQuery() : null),
    async (q) => (q ? gitApi.file(q.sid, q.path) : ""),
  );

  const renderedMd = createMemo(() => {
    if (mode() !== "preview") return "";
    const text = fileRes() ?? "";
    if (!text) return "";
    return renderMd(text);
  });

  const TabButton = (p: { label: string; value: PreviewMode; disabled?: boolean; title?: string }) => (
    <button
      type="button"
      title={p.title}
      disabled={p.disabled}
      onClick={() => !p.disabled && setMode(p.value)}
      class={`px-2.5 h-[22px] text-[11px] rounded transition-colors ${
        p.disabled
          ? "theme-muted opacity-40 cursor-not-allowed"
          : mode() === p.value
          ? "theme-text bg-[var(--ui-accent-soft)]"
          : "theme-muted hover:theme-text hover:bg-[var(--ui-surface-muted)]"
      }`}
    >
      {p.label}
    </button>
  );

  return (
    <div class="flex flex-col h-full overflow-hidden theme-bg">
      <div class="panel-subheader">
        <div class="flex items-center gap-0.5 rounded border theme-border p-0.5">
          <Show when={props.initialMode === "diff"}>
            <TabButton label="Diff" value="diff" disabled={isFolder()} />
          </Show>
          <Show when={pathIsMd()} fallback={<TabButton label="File" value="file" disabled={isFolder()} />}>
            <TabButton label="Preview" value="preview" disabled={isFolder()} title="Rendered Markdown" />
            <TabButton label="Source" value="file" disabled={isFolder()} title="Raw text" />
          </Show>
        </div>
        <span class="font-mono text-[11px] theme-muted truncate min-w-0">{props.path}</span>
        <Show when={props.staged}>
          <span class="chip text-[9px] shrink-0" style={{ "border-color": "rgba(152,195,121,0.4)", "color": "#98c379" }}>staged</span>
        </Show>
        <Show when={props.untracked}>
          <span class="chip text-[9px] shrink-0" style={{ "border-color": "rgba(115,208,255,0.4)", "color": "#73d0ff" }}>untracked</span>
        </Show>
        <div class="ml-auto flex items-center gap-2 shrink-0">
          <Show when={mode() === "diff" && !props.untracked}>
            <label class="flex items-center gap-1 text-[11px] theme-muted cursor-pointer">
              <input
                type="checkbox"
                checked={vsHead()}
                onChange={(e) => setVsHead(e.currentTarget.checked)}
                class="accent-[var(--ui-accent)]"
              />
              vs HEAD
            </label>
          </Show>
          <Show when={props.onAddMention}>
            <button
              type="button"
              onClick={() => props.onAddMention?.(props.path)}
              title="Add to chat as @mention"
              class="icon-btn"
            >
              <AtSign size={13} />
            </button>
          </Show>
        </div>
      </div>

      <div class="flex-1 overflow-hidden min-h-0">
        <Show when={mode() === "diff"}>
          <DiffView diffText={diffRes() ?? ""} loading={diffRes.loading} />
        </Show>

        <Show when={mode() === "file"}>
          <div class="h-full overflow-auto">
            <Show when={fileRes.loading}>
              <div class="px-4 py-3 text-xs theme-muted">Loading file…</div>
            </Show>
            <Show when={fileRes.error}>
              <div class="px-4 py-3 text-xs text-rose-400">{String(fileRes.error)}</div>
            </Show>
            <Show when={!fileRes.loading && !fileRes.error && fileRes() !== undefined}>
              <pre class="font-mono text-[12px] leading-[1.55] whitespace-pre theme-text px-4 py-3">{fileRes()}</pre>
            </Show>
          </div>
        </Show>

        <Show when={mode() === "preview"}>
          <div class="h-full overflow-auto">
            <Show when={fileRes.loading}>
              <div class="px-4 py-3 text-xs theme-muted">Loading…</div>
            </Show>
            <Show when={fileRes.error}>
              <div class="px-4 py-3 text-xs text-rose-400">{String(fileRes.error)}</div>
            </Show>
            <Show when={!fileRes.loading && !fileRes.error}>
              <div class="md-prose px-6 py-5 max-w-[880px]" innerHTML={renderedMd()} />
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
