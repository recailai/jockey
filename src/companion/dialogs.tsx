import { For, Show, createEffect, createResource, createSignal } from "solid-js";
import { ArrowUp, Folder, GitBranch } from "lucide-solid";
import type { SessionSummary } from "../components/types";
import type { Project, RawSession } from "../lib/tauriApi";
import { call } from "../lib/transport";
import { appSessionApi } from "../lib/tauriApi";
import { fmtRelative } from "../lib/formatHelpers";
import {
  Alert,
  AsyncContent,
  Badge,
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogFooter,
  FormField,
  Input,
  ListRow,
  RowButton,
} from "../components/ui";

export function TokenGate(props: { onSubmit: (token: string) => void }) {
  const [value, setValue] = createSignal("");
  return (
    <div class="companion-root items-center justify-center p-6">
      <form
        class="flex w-full max-w-sm flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (value().trim()) props.onSubmit(value().trim());
        }}
      >
        <div>
          <h1 class="text-lg font-semibold">Connect to Jockey</h1>
          <p class="mt-1 text-xs theme-muted">
            Desktop → Settings → Remote Companion
          </p>
        </div>
        <FormField label="Access token">
          <Input
            type="password"
            autocomplete="off"
            autocapitalize="off"
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
          />
        </FormField>
        <Button type="submit" variant="primary" disabled={!value().trim()}>
          Connect
        </Button>
      </form>
    </div>
  );
}

export function ConfirmDialog(props: {
  open: boolean;
  title: string;
  description: string;
  confirmText: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog open={props.open} onOpenChange={(o) => { if (!o) props.onClose(); }}>
      <DialogContent title={props.title} description={props.description}>
        <DialogFooter
          cancelText="Cancel"
          confirmText={props.confirmText}
          confirmVariant="destructive"
          onCancel={props.onClose}
          onConfirm={() => { props.onConfirm(); props.onClose(); }}
        />
      </DialogContent>
    </Dialog>
  );
}

export function ClosedSessionsDialog(props: {
  project: Project | null;
  onClose: () => void;
  onRestored: (session: RawSession) => void;
}) {
  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [rows] = createResource(
    () => props.project?.id ?? null,
    (projectId) => call<SessionSummary[]>("gateway_list_closed_sessions", { projectId }),
  );

  const restore = async (id: string) => {
    setBusy(id);
    setError(null);
    try {
      props.onRestored(await appSessionApi.reopen(id));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog open={props.project !== null} onOpenChange={(o) => { if (!o) props.onClose(); }}>
      <DialogContent
        title={`History · ${props.project?.name ?? ""}`}
      >
        <DialogBody class="max-h-[60dvh] space-y-2 overflow-y-auto">
          <Show when={error()}><Alert tone="danger">{error()}</Alert></Show>
          <AsyncContent
            state={rows.loading ? "loading" : rows.error ? "error" : (rows()?.length ?? 0) === 0 ? "empty" : "ready"}
            error={String(rows.error ?? "")}
            emptyTitle="No closed sessions"
          >
            <For each={rows()}>
              {(s) => (
                <ListRow class="items-center gap-2">
                  <div class="min-w-0 flex-1">
                    <div class="truncate text-sm font-medium">{s.title}</div>
                    <div class="truncate text-xs theme-muted">
                      {s.messageCount} messages · closed {s.closedAt ? fmtRelative(s.closedAt) : ""}
                      {s.preview ? ` · ${s.preview}` : ""}
                    </div>
                  </div>
                  <Button size="sm" variant="primary" disabled={busy() === s.id} onClick={() => void restore(s.id)}>
                    Restore
                  </Button>
                </ListRow>
              )}
            </For>
          </AsyncContent>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

type DirListing = {
  path: string;
  parent: string | null;
  entries: Array<{ name: string; path: string; isGitRepo: boolean; projectId: string | null }>;
};

/** Folder picker over the gateway's directory listing; a phone cannot open a native dialog on the desktop. */
export function DirectoryBrowserDialog(props: {
  open: boolean;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [path, setPath] = createSignal<string | null>(null);
  createEffect(() => { if (props.open) setPath(null); });
  const [listing] = createResource(
    () => (props.open ? { path: path() } : null),
    (key) => call<DirListing>("gateway_list_dirs", { path: key.path }),
  );

  return (
    <Dialog open={props.open} onOpenChange={(o) => { if (!o) props.onClose(); }}>
      <DialogContent title="Choose folder" description={listing()?.path ?? ""}>
        <DialogBody class="max-h-[55dvh] space-y-1 overflow-y-auto">
          <Show when={listing()?.parent}>
            {(parent) => (
              <RowButton onClick={() => setPath(parent())}>
                <span class="flex items-center gap-2 text-sm"><ArrowUp size={14} /> Up</span>
              </RowButton>
            )}
          </Show>
          <AsyncContent
            state={listing.loading ? "loading" : listing.error ? "error" : (listing()?.entries.length ?? 0) === 0 ? "empty" : "ready"}
            error={String(listing.error ?? "")}
            emptyTitle="No subfolders"
          >
            <For each={listing()?.entries}>
              {(entry) => (
                <RowButton onClick={() => setPath(entry.path)}>
                  <span class="flex w-full items-center gap-2 text-left text-sm">
                    <Folder size={14} class="theme-muted" />
                    <span class="min-w-0 flex-1 truncate">{entry.name}</span>
                    <Show when={entry.projectId}><Badge tone="neutral">added</Badge></Show>
                    <Show when={!entry.projectId && entry.isGitRepo}>
                      <Badge tone="info"><GitBranch size={10} /> git</Badge>
                    </Show>
                  </span>
                </RowButton>
              )}
            </For>
          </AsyncContent>
        </DialogBody>
        <DialogFooter
          cancelText="Cancel"
          confirmText="Use this folder"
          confirmDisabled={!listing()?.path}
          onCancel={props.onClose}
          onConfirm={() => { const p = listing()?.path; if (p) { props.onPick(p); props.onClose(); } }}
        />
      </DialogContent>
    </Dialog>
  );
}
