import { For, Show, createEffect, createMemo, createResource, createSignal, on } from "solid-js";
import { ChevronDown, ChevronRight, DownloadCloud, Folder, MessageSquare, RefreshCw, Search } from "lucide-solid";
import {
  Alert,
  AsyncContent,
  Badge,
  Button,
  Checkbox,
  Dialog,
  DialogBody,
  DialogContent,
  DialogFooter,
  IconButton,
  Input,
  SegmentButton,
  SegmentedControl,
  Switch,
} from "./ui";
import type { AsyncContentState } from "./ui";
import { fmtDate, fmtRelative } from "../lib/formatHelpers";
import { projectApi, type ImportableSession, type Project, type RawSession } from "../lib/tauriApi";

export type ImportScope = "project" | "all";

type ImportSessionsModalProps = {
  open: boolean;
  /** Project the "This project" scope lists; without one only "All directories" is offered. */
  project: Project | null;
  initialScope?: ImportScope;
  onClose: () => void;
  onImported: (sessions: RawSession[], createdProjects: Project[]) => void;
  /** Jump to a conversation that is already in Jockey. */
  onOpenExisting?: (appSessionId: string) => void;
};

const AGENTS = ["Claude Code", "Codex", "Pi", "Antigravity", "DeepSeek Harness"] as const;

const shortenPath = (path: string) => path.replace(/^(\/Users|\/home)\/[^/]+/, "~");

type Group = { cwd: string; rows: ImportableSession[]; projectName: string | null | undefined };

export default function ImportSessionsModal(props: ImportSessionsModalProps) {
  const [scope, setScope] = createSignal<ImportScope>("project");
  const [query, setQuery] = createSignal("");
  const [agents, setAgents] = createSignal<Set<string>>(new Set(AGENTS));
  const [showImported, setShowImported] = createSignal(false);
  const [selected, setSelected] = createSignal<Set<string>>(new Set());
  const [collapsed, setCollapsed] = createSignal<Set<string>>(new Set());
  const [cursor, setCursor] = createSignal(0);
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [scanError, setScanError] = createSignal<string | null>(null);
  let listEl: HTMLDivElement | undefined;

  createEffect(
    on(
      () => props.open,
      (open) => {
        if (!open) return;
        setScope(props.initialScope === "all" || !props.project ? "all" : "project");
        setQuery("");
        setShowImported(false);
        setAgents(new Set(AGENTS));
        setCollapsed(new Set<string>());
        setError(null);
      },
    ),
  );

  const [scanned, { mutate }] = createResource(
    () => (props.open ? { scope: scope(), projectId: props.project?.id ?? null } : null),
    async (key): Promise<ImportableSession[]> => {
      setSelected(new Set<string>());
      setScanError(null);
      setCursor(0);
      try {
        if (key.scope === "all") return await projectApi.scanAllImportable();
        return key.projectId ? await projectApi.scanImportable(key.projectId) : [];
      } catch (e) {
        setScanError(`Could not scan CLI sessions: ${String(e)}`);
        return [];
      }
    },
  );

  const refresh = async () => {
    if (scanned.loading) return;
    setSelected(new Set<string>());
    setScanError(null);
    try {
      const rows =
        scope() === "all"
          ? await projectApi.scanAllImportable(true)
          : props.project
            ? await projectApi.scanImportable(props.project.id, true)
            : [];
      mutate(rows);
    } catch (e) {
      setScanError(String(e));
    }
  };

  const filtered = createMemo(() => {
    const needle = query().trim().toLowerCase();
    const allowed = agents();
    return (scanned() ?? [])
      .filter((r) => allowed.has(r.agent))
      .filter((r) => showImported() || !r.imported)
      .filter(
        (r) =>
          !needle ||
          [r.title, r.cwd, r.agent, r.externalSessionId].some((field) => field.toLowerCase().includes(needle)),
      )
      .sort((a, b) => b.lastActiveAt - a.lastActiveAt);
  });

  const groups = createMemo<Group[]>(() => {
    const byCwd = new Map<string, Group>();
    for (const row of filtered()) {
      const group: Group = byCwd.get(row.cwd) ?? { cwd: row.cwd, rows: [], projectName: row.projectName };
      byCwd.set(row.cwd, group);
      group.rows.push(row);
    }
    return [...byCwd.values()];
  });

  const flat = () => scope() === "all";
  // Rows reachable by the keyboard: collapsed groups hide theirs.
  const visibleRows = createMemo(() =>
    flat()
      ? groups().flatMap((g) => (collapsed().has(g.cwd) ? [] : g.rows))
      : filtered(),
  );

  createEffect(() => {
    const max = visibleRows().length - 1;
    if (cursor() > max) setCursor(Math.max(0, max));
  });

  const selectedRows = createMemo(() => (scanned() ?? []).filter((r) => selected().has(r.sourceId)));
  const newProjectCount = createMemo(
    () => new Set(selectedRows().filter((r) => !r.projectId).map((r) => r.cwd)).size,
  );

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const importableIn = (rows: ImportableSession[]) => rows.filter((r) => !r.imported);

  const toggleGroup = (group: Group) => {
    const ids = importableIn(group.rows).map((r) => r.sourceId);
    const all = ids.length > 0 && ids.every((id) => selected().has(id));
    setSelected((prev) => {
      const next = new Set(prev);
      for (const id of ids) {
        if (all) next.delete(id);
        else next.add(id);
      }
      return next;
    });
  };

  const toggleCollapsed = (cwd: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(cwd)) next.delete(cwd);
      else next.add(cwd);
      return next;
    });

  const toggleAgent = (agent: string) =>
    setAgents((prev) => {
      const next = new Set(prev);
      if (next.has(agent)) next.delete(agent);
      else next.add(agent);
      return next;
    });

  const open = (row: ImportableSession) => {
    if (!row.appSessionId) return;
    props.onClose();
    props.onOpenExisting?.(row.appSessionId);
  };

  const handleImport = async () => {
    const ids = [...selected()];
    if (ids.length === 0 || submitting()) return;
    try {
      setSubmitting(true);
      setError(null);
      if (scope() === "all") {
        const result = await projectApi.importCliSessions(ids);
        props.onImported(result.sessions, result.createdProjects);
      } else if (props.project) {
        props.onImported(await projectApi.importSessions(props.project.id, ids), []);
      }
      props.onClose();
    } catch (e) {
      setError(String(e));
      void refresh();
    } finally {
      setSubmitting(false);
    }
  };

  const moveCursor = (delta: number) => {
    const max = visibleRows().length - 1;
    if (max < 0) return;
    const next = Math.min(max, Math.max(0, cursor() + delta));
    setCursor(next);
    const id = visibleRows()[next]?.sourceId;
    queueMicrotask(() =>
      listEl?.querySelector<HTMLElement>(`[data-source-id="${CSS.escape(id ?? "")}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    const typing = target instanceof HTMLInputElement && target.type === "text";
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveCursor(1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      moveCursor(-1);
    } else if (e.key === " " && (!typing || query() === "")) {
      const row = visibleRows()[cursor()];
      if (!row || target instanceof HTMLButtonElement) return;
      e.preventDefault();
      if (!row.imported) toggle(row.sourceId);
    } else if (e.key === "Enter" && !(target instanceof HTMLButtonElement)) {
      e.preventDefault();
      void handleImport();
    }
  };

  const state = (): AsyncContentState => {
    if (scanned.loading) return "loading";
    if (scanError()) return "error";
    if (filtered().length === 0) return "empty";
    return "ready";
  };

  const renderRow = (row: ImportableSession) => {
    const isSelected = () => selected().has(row.sourceId);
    const isCursor = () => visibleRows()[cursor()]?.sourceId === row.sourceId;
    return (
      <div
        role="option"
        aria-selected={isSelected()}
        data-source-id={row.sourceId}
        onClick={() => {
          setCursor(visibleRows().findIndex((r) => r.sourceId === row.sourceId));
          if (row.imported) open(row);
          else toggle(row.sourceId);
        }}
        class={`flex items-center gap-3 px-3 py-2 cursor-pointer select-none hover:bg-[var(--ui-row-hover)] ${
          isSelected() ? "bg-[var(--ui-accent-soft)]" : ""
        } ${isCursor() ? "outline outline-1 -outline-offset-1 outline-[var(--ui-accent)]" : ""}`}
      >
        <div class="w-4 shrink-0" onClick={(e) => e.stopPropagation()}>
          <Show when={!row.imported}>
            <Checkbox checked={isSelected()} onChange={() => toggle(row.sourceId)} />
          </Show>
        </div>
        <div class="min-w-0 flex-1 space-y-0.5">
          <div class="text-[12.5px] font-semibold theme-text truncate">{row.title}</div>
          <Show when={!flat()}>
            <div class="text-[11px] theme-muted truncate" title={row.cwd}>
              {shortenPath(row.cwd)}
            </div>
          </Show>
        </div>
        <Badge tone="neutral" variant="outline" label={row.agent} />
        {/* ACP-only conversations (Antigravity) have no local transcript to count. */}
        <Show when={row.turnCount > 0} fallback={
          <span class="text-[11px] theme-muted shrink-0" title="History stays with the agent; importing continues the conversation">no local history</span>
        }>
          <span class="inline-flex items-center gap-1 text-[11px] theme-muted shrink-0">
            <MessageSquare size={10} />
            {row.turnCount}
          </span>
        </Show>
        <span class="text-[11px] theme-muted shrink-0 w-14 text-right" title={fmtDate(row.lastActiveAt)}>
          {fmtRelative(row.lastActiveAt)}
        </span>
        <Show when={row.imported}>
          <Badge tone="success" variant="subtle" label="In Jockey" />
          <Button
            size="sm"
            variant="ghost"
            onClick={(e) => {
              e.stopPropagation();
              open(row);
            }}
          >
            Open
          </Button>
        </Show>
      </div>
    );
  };

  return (
    <Dialog open={props.open} onOpenChange={(o) => { if (!o) props.onClose(); }}>
      <DialogContent
        class="max-w-[720px]"
        title="Import CLI Sessions"
        description="Bring conversations from Claude Code, Codex, Pi, Antigravity and DeepSeek Harness into Jockey"
        icon={<DownloadCloud size={20} class="text-[var(--ui-accent)]" />}
      >
        <DialogBody class="space-y-3" onKeyDown={onKeyDown}>
          <Show when={error()}>
            <Alert tone="danger" onClose={() => setError(null)}>
              {error()}
            </Alert>
          </Show>

          <div class="flex flex-wrap items-center gap-2">
            <SegmentedControl>
              <SegmentButton
                active={scope() === "project"}
                disabled={!props.project}
                onClick={() => setScope("project")}
              >
                This project
              </SegmentButton>
              <SegmentButton active={scope() === "all"} onClick={() => setScope("all")}>
                All directories
              </SegmentButton>
            </SegmentedControl>
            <div class="relative min-w-[180px] flex-1">
              <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 theme-muted pointer-events-none" />
              <Input
                type="text"
                autofocus
                placeholder="Search title, directory, agent, session id"
                class="pl-7"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
              />
            </div>
            <IconButton title="Rescan" aria-label="Rescan" onClick={() => void refresh()} disabled={scanned.loading}>
              <RefreshCw size={14} />
            </IconButton>
          </div>

          <div class="flex flex-wrap items-center justify-between gap-2">
            <SegmentedControl>
              <For each={AGENTS}>
                {(agent) => (
                  <SegmentButton active={agents().has(agent)} onClick={() => toggleAgent(agent)}>
                    {agent === "Claude Code" ? "Claude" : agent === "DeepSeek Harness" ? "DeepSeek" : agent}
                  </SegmentButton>
                )}
              </For>
            </SegmentedControl>
            <Switch checked={showImported()} onChange={setShowImported} label="Show already imported" />
          </div>

          <div
            ref={listEl}
            role="listbox"
            aria-multiselectable="true"
            class="max-h-[380px] min-h-[160px] overflow-y-auto rounded-lg border border-[var(--ui-border)] bg-[var(--ui-card-bg)]"
          >
            <AsyncContent
              state={state()}
              error={scanError()}
              onRetry={() => void refresh()}
              loadingFallback={
                <div class="py-12 flex flex-col items-center justify-center gap-2 text-xs theme-muted">
                  <span class="inline-block animate-spin text-sm">⟳</span>
                  <span>
                    {scope() === "all"
                      ? "Scanning every CLI session on this machine…"
                      : "Scanning this project's CLI sessions…"}
                  </span>
                </div>
              }
              emptyTitle={(scanned() ?? []).length === 0 ? "No CLI sessions found" : "Nothing matches"}
              emptyDescription={
                (scanned() ?? []).length === 0
                  ? scope() === "all"
                    ? "No Claude Code, Codex, Pi or Antigravity conversations were found on this machine."
                    : "No CLI sessions were found for this project directory."
                  : "Adjust the search or filters, or show already imported sessions."
              }
            >
              <Show when={flat()} fallback={<div class="divide-y divide-[var(--ui-border)]"><For each={filtered()}>{renderRow}</For></div>}>
                <For each={groups()}>
                  {(group) => {
                    const importable = () => importableIn(group.rows);
                    const picked = () => importable().filter((r) => selected().has(r.sourceId)).length;
                    const isCollapsed = () => collapsed().has(group.cwd);
                    return (
                      <div>
                        <div class="sticky top-0 z-10 flex items-center gap-2 px-3 py-1.5 border-b border-[var(--ui-border)] bg-[var(--ui-surface-muted)]">
                          <button
                            type="button"
                            class="p-0.5 theme-muted"
                            aria-label={isCollapsed() ? "Expand" : "Collapse"}
                            onClick={() => toggleCollapsed(group.cwd)}
                          >
                            {isCollapsed() ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
                          </button>
                          <div class="w-4 shrink-0">
                            <Show when={importable().length > 0}>
                              <Checkbox
                                checked={picked() === importable().length}
                                indeterminate={picked() > 0 && picked() < importable().length}
                                onChange={() => toggleGroup(group)}
                              />
                            </Show>
                          </div>
                          <Folder size={12} class="shrink-0 theme-muted" />
                          <span class="min-w-0 flex-1 truncate font-mono text-[11px] theme-text" title={group.cwd}>
                            {shortenPath(group.cwd)}
                          </span>
                          <span class="text-[11px] theme-muted shrink-0">{group.rows.length}</span>
                          <Show
                            when={group.projectName}
                            fallback={<Badge tone="info" variant="subtle" label="new project" />}
                          >
                            <Badge tone="neutral" variant="outline" label={`→ ${group.projectName}`} />
                          </Show>
                        </div>
                        <Show when={!isCollapsed()}>
                          <div class="divide-y divide-[var(--ui-border)]">
                            <For each={group.rows}>{renderRow}</For>
                          </div>
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </Show>
            </AsyncContent>
          </div>
        </DialogBody>

        <DialogFooter>
          <span class="mr-auto text-[11px] theme-muted">
            {selected().size} selected
            <Show when={newProjectCount() > 0}>
              {` · will create ${newProjectCount()} ${newProjectCount() === 1 ? "project" : "projects"}`}
            </Show>
          </span>
          <Button variant="secondary" onClick={() => props.onClose()}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={selected().size === 0 || submitting()}
            onClick={() => void handleImport()}
          >
            {submitting() ? "Importing…" : "Import"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
