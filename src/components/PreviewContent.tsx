import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { AtSign } from "lucide-solid";
import { gitApi, fsApi } from "../lib/tauriApi";
import { renderMd } from "../lib/markdown";
import type { PreviewMode } from "./types";
import DiffView from "./DiffView";

function FileWithLineNumbers(props: { text: string }) {
  const lines = () => props.text.split("\n");
  const lineCount = () => lines().length;
  const gutterWidth = () => String(lineCount()).length;
  return (
    <div class="flex font-mono text-[12px] leading-[1.55] py-3 min-w-0">
      <div
        class="shrink-0 select-none text-right pr-4 theme-muted border-r theme-border"
        style={{ "min-width": `${gutterWidth() * 0.6 + 1.5}em`, "padding-left": "1rem" }}
      >
        <For each={lines()}>
          {(_, i) => <div>{i() + 1}</div>}
        </For>
      </div>
      <pre class="flex-1 overflow-x-auto whitespace-pre theme-text pl-4 pr-4">{props.text}</pre>
    </div>
  );
}

type PreviewContentProps = {
  appSessionId: () => string | undefined;
  cwd: string;
  path: string;
  initialMode: PreviewMode;
  staged: boolean;
  untracked: boolean;
  version: () => number;
  onAddMention?: (path: string) => void;
};

const MD_EXT = /\.(md|markdown|mdx)$/i;
const IMG_EXT = /\.(png|jpg|jpeg|gif|webp|svg|bmp|ico|tiff|avif)$/i;

const isMarkdown = (p: string) => MD_EXT.test(p);
const isImage = (p: string) => IMG_EXT.test(p);

function imgMimeType(p: string): string {
  const ext = p.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg",
    gif: "image/gif", webp: "image/webp", svg: "image/svg+xml",
    bmp: "image/bmp", ico: "image/x-icon", tiff: "image/tiff",
    avif: "image/avif",
  };
  return map[ext] ?? "image/png";
}

function resolveImgPath(src: string, mdPath: string): string | null {
  if (!src || src.startsWith("http://") || src.startsWith("https://") || src.startsWith("data:")) {
    return null;
  }
  if (src.startsWith("/")) {
    return src.slice(1);
  }
  const dir = mdPath.includes("/") ? mdPath.slice(0, mdPath.lastIndexOf("/") + 1) : "";
  const parts = (dir + src).split("/");
  const resolved: string[] = [];
  for (const p of parts) {
    if (p === "..") resolved.pop();
    else if (p && p !== ".") resolved.push(p);
  }
  return resolved.join("/");
}

async function hydrateImages(
  container: HTMLElement,
  appSessionId: string | undefined,
  mdPath: string,
): Promise<void> {
  const imgs = container.querySelectorAll<HTMLImageElement>("img[src]");
  await Promise.all(
    Array.from(imgs).map(async (img) => {
      const src = img.getAttribute("src") ?? "";
      const relPath = resolveImgPath(src, mdPath);
      if (!relPath) return;
      if (!IMG_EXT.test(relPath)) return;
      try {
        const b64 = await fsApi.readFileBase64(appSessionId, relPath);
        img.src = `data:${imgMimeType(relPath)};base64,${b64}`;
      } catch {
      }
    }),
  );
}

export default function PreviewContent(props: PreviewContentProps) {
  const isFolder = () => props.path.endsWith("/");
  const pathIsMd = createMemo(() => isMarkdown(props.path));
  const pathIsImg = createMemo(() => isImage(props.path));

  const computeInitial = (): PreviewMode => {
    if (props.initialMode === "diff") return "diff";
    if (pathIsImg()) return "image";
    return pathIsMd() ? "preview" : "file";
  };
  const [mode, setMode] = createSignal<PreviewMode>(computeInitial());
  const [vsHead, setVsHead] = createSignal(false);

  let mdContainerRef: HTMLDivElement | undefined;
  let hydrateAbort: AbortController | null = null;

  onCleanup(() => {
    hydrateAbort?.abort();
  });

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

  const [imgRes] = createResource(
    () => (mode() === "image" ? fileQuery() : null),
    async (q) => {
      if (!q) return "";
      return fsApi.readFileBase64(q.sid, q.path);
    },
  );

  const imgSrc = createMemo(() => {
    const b64 = imgRes();
    if (!b64) return "";
    return `data:${imgMimeType(props.path)};base64,${b64}`;
  });

  const renderedMd = createMemo(() => {
    if (mode() !== "preview") return "";
    const text = fileRes() ?? "";
    if (!text) return "";
    return renderMd(text);
  });

  createEffect(() => {
    const html = renderedMd();
    if (!html) return;
    const el = mdContainerRef;
    if (!el) return;
    hydrateAbort?.abort();
    hydrateAbort = new AbortController();
    void hydrateImages(el, props.appSessionId(), props.path).catch(() => {});
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
          <Show when={pathIsImg()}>
            <TabButton label="Image" value="image" disabled={isFolder()} />
          </Show>
          <Show when={pathIsMd()} fallback={
            <Show when={!pathIsImg()}>
              <TabButton label="File" value="file" disabled={isFolder()} />
            </Show>
          }>
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

        <Show when={mode() === "image"}>
          <div class="h-full overflow-auto flex items-center justify-center p-4">
            <Show when={imgRes.loading}>
              <div class="text-xs theme-muted">Loading image…</div>
            </Show>
            <Show when={imgRes.error}>
              <div class="text-xs text-rose-400">{String(imgRes.error)}</div>
            </Show>
            <Show when={!imgRes.loading && !imgRes.error && imgSrc()}>
              <img
                src={imgSrc()}
                alt={props.path}
                class="max-w-full max-h-full object-contain rounded"
              />
            </Show>
          </div>
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
              <FileWithLineNumbers text={fileRes()!} />
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
              <div
                ref={mdContainerRef}
                class="md-prose px-6 py-5 max-w-[880px]"
                innerHTML={renderedMd()}
              />
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
