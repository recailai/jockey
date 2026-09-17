import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import { Bot, DownloadCloud, Folder, MessageSquare, RefreshCw } from "lucide-solid";
import { Alert, Badge, Button, Checkbox, Dialog, DialogBody, DialogContent, DialogFooter, IconButton } from "./ui";
import { fmtDate, fmtRelative } from "../lib/formatHelpers";
import { projectApi, type ImportableSession, type Project, type RawSession } from "../lib/tauriApi";

type ImportSessionsModalProps = {
  open: boolean;
  project: Project | null;
  onClose: () => void;
  onImported: (sessions: RawSession[]) => void;
};

export default function ImportSessionsModal(props: ImportSessionsModalProps) {
  const [selected, setSelected] = createSignal<Set<string>>(new Set());
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const [scanned, { mutate }] = createResource(
    () => (props.open && props.project ? props.project.id : null),
    async (projectId): Promise<ImportableSession[]> => {
      setSelected(new Set<string>());
      setError(null);
      try {
        return await projectApi.scanImportable(projectId);
      } catch (e) {
        setError(`Could not scan CLI sessions: ${String(e)}`);
        return [];
      }
    },
  );

  const rows = () => scanned() ?? [];
  // Default selection to only unimported sessions
  const importable = createMemo(() => rows().filter((r) => !r.imported));
  const allNewSelected = () =>
    importable().length > 0 && importable().every((r) => selected().has(r.sourceId));

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    setSelected(allNewSelected() ? new Set<string>() : new Set(importable().map((r) => r.sourceId)));
  };

  const refresh = async () => {
    const project = props.project;
    if (!project || scanned.loading) return;
    setSelected(new Set<string>());
    setError(null);
    try {
      mutate(await projectApi.scanImportable(project.id, true));
    } catch (e) {
      setError(String(e));
    }
  };

  const handleImport = async () => {
    const project = props.project;
    const ids = [...selected()];
    if (!project || ids.length === 0) return;
    try {
      setSubmitting(true);
      setError(null);
      const imported = await projectApi.importSessions(project.id, ids);
      props.onImported(imported);
      props.onClose();
    } catch (e) {
      setError(String(e));
      void refresh();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={props.open} onOpenChange={(open) => { if (!open) props.onClose(); }}>
      <DialogContent
        class="max-w-[580px]"
        title="Import CLI Sessions"
        description={
          props.project
            ? `Sessions found for "${props.project.name}" across installed CLI agents`
            : "Sessions from installed CLI agents"
        }
        icon={<DownloadCloud size={20} class="text-[var(--ui-accent)]" />}
      >
        <DialogBody class="space-y-3">
          <Show when={error()}>
            <Alert tone="danger" onClose={() => setError(null)}>
              {error()}
            </Alert>
          </Show>

          <Show
            when={!scanned.loading}
            fallback={
              <div class="py-12 flex flex-col items-center justify-center gap-2 text-xs theme-muted">
                <span class="inline-block animate-spin text-sm">⟳</span>
                <span>Scanning local agent directories…</span>
              </div>
            }
          >
            <Show
              when={rows().length > 0}
              fallback={
                <div class="py-10 text-center text-xs theme-muted">
                  No importable CLI sessions found for this workspace directory.
                </div>
              }
            >
              <div class="flex items-center justify-between pb-1">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={toggleAll}
                  disabled={importable().length === 0}
                  class="text-[11px] h-7 px-2"
                >
                  {allNewSelected() ? "Deselect all" : `Select all ${importable().length} new`}
                </Button>
                <div class="flex items-center gap-2">
                  <span class="text-[11px] theme-muted">
                    {rows().length} found · {rows().length - importable().length} imported
                  </span>
                  <IconButton
                    size="sm"
                    variant="ghost"
                    onClick={() => void refresh()}
                    title="Rescan workspace"
                  >
                    <RefreshCw size={12} />
                  </IconButton>
                </div>
              </div>

              <div class="max-h-[340px] overflow-y-auto rounded-lg border border-[var(--ui-border)] divide-y divide-[var(--ui-border)] bg-[var(--ui-card-bg)]">
                <For each={rows()}>
                  {(row) => {
                    const isSelected = () => selected().has(row.sourceId);
                    return (
                      <div
                        role="button"
                        tabIndex={0}
                        onClick={() => toggle(row.sourceId)}
                        class={`flex items-start gap-3 p-3 text-left transition-colors cursor-pointer select-none hover:bg-[var(--ui-row-hover)] ${
                          isSelected() ? "bg-[var(--ui-accent-soft)]" : ""
                        }`}
                      >
                        <div class="pt-0.5" onClick={(e) => e.stopPropagation()}>
                          <Checkbox
                            checked={isSelected()}
                            onChange={() => toggle(row.sourceId)}
                          />
                        </div>

                        <div class="min-w-0 flex-1 space-y-1">
                          <div class="flex items-center justify-between gap-2">
                            <span class="text-[12.5px] font-semibold theme-text truncate">
                              {row.title}
                            </span>
                            <Show when={row.imported}>
                              <Badge tone="neutral" variant="subtle" label="Imported" />
                            </Show>
                          </div>

                          <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] theme-muted">
                            <span class="inline-flex items-center gap-1 font-medium text-[var(--ui-text)]">
                              <Bot size={11} class="text-[var(--ui-accent)] opacity-80" />
                              <span>{row.agent}</span>
                            </span>

                            <span class="inline-flex items-center gap-1">
                              <MessageSquare size={10} />
                              <span>{row.turnCount} {row.turnCount === 1 ? "turn" : "turns"}</span>
                            </span>

                            <span title={fmtDate(row.lastActiveAt)}>
                              {fmtRelative(row.lastActiveAt)}
                            </span>

                            <Show when={row.cwd}>
                              <span class="inline-flex items-center gap-1 truncate max-w-[200px]" title={row.cwd}>
                                <Folder size={10} class="shrink-0 opacity-60" />
                                <span class="truncate">{row.cwd}</span>
                              </span>
                            </Show>
                          </div>
                        </div>
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
          </Show>
        </DialogBody>

        <DialogFooter
          cancelText="Cancel"
          confirmText={submitting() ? "Importing…" : `Import ${selected().size > 0 ? `(${selected().size})` : ""}`.trim()}
          confirmDisabled={selected().size === 0 || submitting()}
          confirmLoading={submitting()}
          onCancel={() => props.onClose()}
          onConfirm={() => void handleImport()}
        />
      </DialogContent>
    </Dialog>
  );
}
