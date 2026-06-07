import { For, Show, createEffect, createMemo, createResource, createSignal } from "solid-js";
import { Copy } from "lucide-solid";
import { gitApi, type GitCommitFileEntry } from "../lib/tauriApi";
import DiffView from "./DiffView";
import FileGlyph from "./FileGlyph";
import { IconButton, ListRow } from "./ui";

type CommitPreviewContentProps = {
  appSessionId: () => string | undefined;
  commitOid: string;
  version: () => number;
};

function fmtRelative(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  if (diff < 604_800_000) return `${Math.floor(diff / 86_400_000)}d ago`;
  return new Date(ts).toLocaleDateString();
}

function statusClass(entry: GitCommitFileEntry): string {
  switch (entry.statusLetter) {
    case "A": return "git-status-added";
    case "D": return "git-status-deleted";
    case "R":
    case "C": return "git-status-renamed";
    default: return "git-status-modified";
  }
}

function statusLetter(entry: GitCommitFileEntry): string {
  switch (entry.statusLetter) {
    case "A": return "A";
    case "D": return "D";
    case "R":
    case "C": return "R";
    default: return "M";
  }
}

export default function CommitPreviewContent(props: CommitPreviewContentProps) {
  const [selectedPath, setSelectedPath] = createSignal<string | null>(null);

  const detailQuery = createMemo(() => ({
    sid: props.appSessionId(),
    oid: props.commitOid,
    v: props.version(),
  }));

  const [detailRes] = createResource(detailQuery, async (q) => {
    if (!q.oid) return null;
    return gitApi.commitDetail(q.sid, q.oid);
  });

  createEffect(() => {
    const detail = detailRes();
    if (!detail) return;
    const current = selectedPath();
    if (current && detail.files.some((f) => f.path === current)) return;
    setSelectedPath(detail.files[0]?.path ?? null);
  });

  const diffQuery = createMemo(() => ({
    sid: props.appSessionId(),
    oid: props.commitOid,
    path: selectedPath(),
    v: props.version(),
  }));

  const [diffRes] = createResource(
    () => (diffQuery().path ? diffQuery() : null),
    async (q) => (q?.path ? gitApi.commitFileDiff(q.sid, q.oid, q.path) : ""),
  );

  const copyOid = async () => {
    try {
      await navigator.clipboard?.writeText(props.commitOid);
    } catch {}
  };

  return (
    <div class="commit-preview flex flex-col h-full overflow-hidden theme-bg">
      <Show when={detailRes.loading}>
        <div class="px-4 py-3 text-xs theme-muted">Loading commit…</div>
      </Show>
      <Show when={detailRes.error}>
        <div class="preview-error-text px-4 py-3">{String(detailRes.error)}</div>
      </Show>
      <Show when={detailRes()}>
        {(detail) => (
          <div class="commit-preview-body flex flex-1 min-h-0 overflow-hidden">
            <aside class="commit-preview-sidebar">
              <div class="commit-preview-header">
                <div class="commit-preview-summary">{detail().summary}</div>
                <div class="commit-preview-meta">
                  <span class="truncate">{detail().authorName}</span>
                  <span>{fmtRelative(detail().committedAt)}</span>
                </div>
                <div class="commit-preview-hash">
                  <span class="font-mono">{detail().shortOid}</span>
                  <IconButton size="sm" onClick={() => { void copyOid(); }} title="Copy full hash">
                    <Copy size={12} />
                  </IconButton>
                  <Show when={detail().additions > 0 || detail().deletions > 0}>
                    <span class="commit-preview-stats">
                      <span class="commit-stat-add">+{detail().additions}</span>
                      <span class="commit-stat-del">-{detail().deletions}</span>
                    </span>
                  </Show>
                </div>
                <div class="commit-preview-file-count">
                  {detail().files.length} changed {detail().files.length === 1 ? "file" : "files"}
                </div>
              </div>
              <div class="commit-preview-files">
                <For each={detail().files}>
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
                        class="commit-preview-file-row"
                        classList={{ "is-active": selectedPath() === entry.path }}
                        onClick={() => setSelectedPath(entry.path)}
                        title={entry.oldPath ? `${entry.oldPath} → ${entry.path}` : entry.path}
                      >
                        <FileGlyph name={basename()} />
                        <div class="min-w-0 flex-1">
                          <div class="truncate">{basename()}</div>
                          <Show when={dirname()}>
                            <div class="text-[10px] theme-muted truncate">{dirname()}</div>
                          </Show>
                          <Show when={entry.oldPath}>
                            <div class="text-[10px] theme-muted truncate">from {entry.oldPath}</div>
                          </Show>
                        </div>
                        <span class={`git-status ${statusClass(entry)}`}>{statusLetter(entry)}</span>
                      </ListRow>
                    );
                  }}
                </For>
                <Show when={detail().files.length === 0}>
                  <div class="px-3 py-2 text-[11px] theme-muted">No file changes in this commit.</div>
                </Show>
              </div>
            </aside>
            <section class="commit-preview-diff min-w-0 flex-1 flex flex-col overflow-hidden">
              <div class="commit-preview-diff-header">
                <span class="font-mono text-[11px] theme-muted truncate">
                  {selectedPath() ?? "Select a file"}
                </span>
              </div>
              <div class="flex-1 min-h-0">
                <Show
                  when={selectedPath()}
                  fallback={<div class="px-4 py-3 text-xs theme-muted">Select a file to view its diff.</div>}
                >
                  <DiffView diffText={diffRes() ?? ""} loading={diffRes.loading} emptyLabel="No diff for this file" />
                </Show>
              </div>
            </section>
          </div>
        )}
      </Show>
    </div>
  );
}
