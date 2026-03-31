import { For, Show, createEffect, createMemo, createSignal, onMount } from "solid-js";
import type { AppSession } from "../types";
import { RUNTIME_COLOR } from "../types";
import { Badge, EmptyState, FieldRow, ActionButton } from "./primitives";
import type { ContextEntry, StoredSession } from "./primitives";
import { fmtDate, fmtRelative } from "./primitives";
import { appSessionApi, commandApi } from "../../lib/tauriApi";

// ─── value renderer ──────────────────────────────────────────────────────────
function ContextValue(props: { raw: string }) {
  const parsed = () => {
    try { return JSON.parse(props.raw); } catch { return null; }
  };
  return (
    <Show when={parsed()} fallback={
      <span class="font-mono text-[10px] theme-text whitespace-pre-wrap break-all">{props.raw}</span>
    }>
      {(v) => (
        <Show when={Array.isArray(v())} fallback={
          <pre class="whitespace-pre-wrap break-all font-mono text-[10px] theme-text leading-relaxed">
            {JSON.stringify(v(), null, 2)}
          </pre>
        }>
          <ul class="space-y-0.5">
            <For each={v() as unknown[]}>
              {(item) => (
                <li class="flex items-start gap-1.5">
                  <span class="mt-[3px] h-1 w-1 shrink-0 rounded-full bg-amber-400/60" />
                  <span class="font-mono text-[10px] theme-text break-all">
                    {typeof item === "string" ? item : JSON.stringify(item)}
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      )}
    </Show>
  );
}

// ─── context section ─────────────────────────────────────────────────────────
function SessionContextSection(props: { sessionId: string }) {
  const [entries, setEntries] = createSignal<ContextEntry[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [editingKey, setEditingKey] = createSignal<string | null>(null);
  const [editValue, setEditValue] = createSignal("");
  const [addingNew, setAddingNew] = createSignal(false);
  const [newKey, setNewKey] = createSignal("");
  const [newValue, setNewValue] = createSignal("");
  let reqSeq = 0;

  const reload = () => {
    const sid = props.sessionId;
    const seq = ++reqSeq;
    if (!sid) { setEntries([]); return; }
    setLoading(true);
    commandApi.apply<{ entries?: ContextEntry[] }>("/app_context list", sid)
      .then((res) => { if (seq === reqSeq) setEntries(res.payload.entries ?? []); })
      .catch(() => { if (seq === reqSeq) setEntries([]); })
      .finally(() => { if (seq === reqSeq) setLoading(false); });
  };

  createEffect(() => {
    props.sessionId; // reactive dependency
    reload();
  });

  const handleSet = async (key: string, value: string) => {
    if (!key.trim() || !value.trim()) return;
    await commandApi.apply(`/app_context set ${key} ${value}`, props.sessionId).catch(() => {});
    setEditingKey(null);
    reload();
  };

  const handleDelete = async (key: string) => {
    await commandApi.apply(`/app_context delete ${key}`, props.sessionId).catch(() => {});
    reload();
  };

  const handleAdd = async () => {
    const k = newKey().trim();
    const v = newValue().trim();
    if (!k || !v) return;
    await handleSet(k, v);
    setNewKey(""); setNewValue(""); setAddingNew(false);
  };

  return (
    <div>
      {/* Section header */}
      <div class="mb-2 flex items-center justify-between">
        <span class="font-mono text-[10px] font-semibold uppercase tracking-widest theme-muted">
          Context
          <Show when={!loading() && entries().length > 0}>
            <span class="ml-1.5 theme-muted opacity-60">({entries().length})</span>
          </Show>
          <Show when={loading()}>
            <span class="ml-1.5 theme-muted italic opacity-60">loading…</span>
          </Show>
        </span>
        <div class="flex items-center gap-1.5">
          <button
            onClick={reload}
            class="font-mono text-[9px] theme-muted hover:theme-text transition-colors"
            title="Refresh"
          >↻</button>
          <button
            onClick={() => { setAddingNew(true); setNewKey(""); setNewValue(""); }}
            class="font-mono text-[9px] text-amber-500/70 hover:text-amber-400 transition-colors"
          >+ add</button>
        </div>
      </div>

      {/* Add row */}
      <Show when={addingNew()}>
        <div class="mb-2 flex flex-col gap-1.5 rounded-md border border-amber-500/20 bg-[var(--ui-surface-muted)] p-2">
          <input
            value={newKey()}
            onInput={(e) => setNewKey(e.currentTarget.value)}
            placeholder="key"
            class="h-6 w-full rounded border border-theme theme-surface px-2 font-mono text-[10px] theme-text placeholder:text-[var(--ui-muted)] focus:border-[var(--ui-border-strong)] focus:outline-none"
          />
          <textarea
            value={newValue()}
            onInput={(e) => setNewValue(e.currentTarget.value)}
            placeholder='value (string or JSON: "text", [...], {...})'
            rows={2}
            class="w-full resize-none rounded border border-zinc-700 bg-zinc-950 px-2 py-1 font-mono text-[10px] text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none"
            onKeyDown={(e) => { if (e.key === "Escape") setAddingNew(false); }}
          />
          <div class="flex gap-1.5">
            <ActionButton label="Save" variant="primary" onClick={() => void handleAdd()} />
            <ActionButton label="Cancel" variant="ghost" onClick={() => setAddingNew(false)} />
          </div>
        </div>
      </Show>

      {/* Empty */}
      <Show when={!loading() && entries().length === 0 && !addingNew()}>
        <p class="font-mono text-[10px] theme-muted italic opacity-60">no context entries</p>
      </Show>

      {/* Entries */}
      <Show when={entries().length > 0}>
        <div class="space-y-1">
          <For each={entries()}>
            {(entry) => {
              const isEditing = () => editingKey() === entry.key;
              return (
                <div class="group rounded border theme-border theme-panel px-3 py-2 hover:border-[var(--ui-border-strong)] hover:bg-[var(--ui-accent-soft)] transition-colors">
                  <div class="mb-1 flex items-center justify-between gap-2">
                    <span class="font-mono text-[9px] font-semibold text-amber-400/80 truncate">{entry.key}</span>
                    <div class="flex items-center gap-2 shrink-0">
                      <Show when={!isEditing()}>
                        <button
                          onClick={() => { setEditingKey(entry.key); setEditValue(entry.value); }}
                          class="opacity-0 group-hover:opacity-100 font-mono text-[9px] theme-muted hover:theme-text transition-all"
                        >edit</button>
                      </Show>
                      <button
                        onClick={() => void handleDelete(entry.key)}
                        class="opacity-0 group-hover:opacity-100 font-mono text-[9px] text-rose-600 hover:text-rose-400 transition-all"
                      >del</button>
                    </div>
                  </div>
                  <Show when={isEditing()} fallback={<ContextValue raw={entry.value} />}>
                    <textarea
                      value={editValue()}
                      onInput={(e) => setEditValue(e.currentTarget.value)}
                      rows={3}
                      class="w-full resize-none rounded border border-zinc-600 bg-zinc-950 px-2 py-1 font-mono text-[10px] text-zinc-200 focus:border-zinc-400 focus:outline-none"
                      onKeyDown={(e) => { if (e.key === "Escape") setEditingKey(null); }}
                    />
                    <div class="mt-1 flex gap-1.5">
                      <ActionButton label="Save" variant="primary" onClick={() => void handleSet(entry.key, editValue())} />
                      <ActionButton label="Cancel" variant="ghost" onClick={() => setEditingKey(null)} />
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}

// ─── main component ───────────────────────────────────────────────────────────
export function SessionsTab(props: {
  activeSessions: AppSession[];
  onRestoreSession?: (id: string, title: string, activeRole: string, runtimeKind: string | null, cwd: string | null) => void;
}) {
  const [storedSessions, setStoredSessions] = createSignal<StoredSession[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [reopening, setReopening] = createSignal<string | null>(null);
  const [search, setSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);

  type RawSession = {
    id: string; title: string; activeRole?: string;
    runtimeKind?: string | null; cwd?: string | null;
    messages?: unknown[]; createdAt?: number; lastActiveAt?: number; closedAt?: number | null;
  };

  const mapRaw = (r: RawSession, closedAt: number | null): StoredSession => ({
    id: r.id,
    title: r.title,
    activeRole: r.activeRole ?? "—",
    runtimeKind: r.runtimeKind ?? null,
    cwd: r.cwd ?? null,
    messageCount: Array.isArray(r.messages) ? r.messages.length : 0,
    createdAt: r.createdAt ?? 0,
    updatedAt: r.lastActiveAt ?? r.createdAt ?? 0,
    closedAt,
  });

  const load = async () => {
    setLoading(true);
    try {
      const [active, closed] = await Promise.all([
        appSessionApi.list(),
        appSessionApi.listClosed(),
      ]);
      setStoredSessions([
        ...active.map((r) => mapRaw(r, null)),
        ...closed.map((r) => mapRaw(r, r.closedAt ?? null)),
      ]);
    } catch { /* ignore */ }
    setLoading(false);
  };

  onMount(() => void load());

  const filtered = createMemo(() => {
    const q = search().toLowerCase();
    const matches = storedSessions().filter((s) =>
      !q || s.title.toLowerCase().includes(q) || (s.cwd ?? "").toLowerCase().includes(q),
    );
    return matches.sort((a, b) => {
      const aClosed = a.closedAt !== null;
      const bClosed = b.closedAt !== null;
      if (aClosed !== bClosed) return aClosed ? 1 : -1;
      return (b.updatedAt || b.createdAt) - (a.updatedAt || a.createdAt);
    });
  });

  const selected = createMemo(() =>
    storedSessions().find((s) => s.id === selectedId()) ?? null,
  );

  const activeIds = () => new Set(props.activeSessions.map((s) => s.id));

  const handleReopen = async (s: StoredSession) => {
    if (reopening()) return;
    setReopening(s.id);
    try {
      await appSessionApi.reopen(s.id);
      props.onRestoreSession?.(s.id, s.title, s.activeRole, s.runtimeKind, s.cwd);
      setSelectedId(s.id);
      setStoredSessions((prev) => prev.map((p) => p.id === s.id ? { ...p, closedAt: null, updatedAt: Date.now() } : p));
    } catch { /* ignore */ } finally {
      setReopening(null);
    }
  };

  return (
    <div class="flex h-full">
      {/* List pane */}
      <div class="flex w-64 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <input
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
            placeholder="Filter sessions…"
            class="h-7 w-full rounded-md border theme-border theme-surface px-2.5 font-mono text-[10px] theme-text placeholder:text-[var(--ui-muted)] focus:border-[var(--ui-border-strong)] focus:outline-none"
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={loading()}>
            <p class="p-4 font-mono text-[10px] theme-muted opacity-60">Loading…</p>
          </Show>
          <Show when={!loading() && filtered().length === 0}>
            <p class="p-4 font-mono text-[10px] theme-muted opacity-60">No sessions found.</p>
          </Show>
          <For each={filtered()}>
            {(s) => {
              const isActive = () => activeIds().has(s.id);
              const isClosed = () => s.closedAt !== null && !activeIds().has(s.id);
              const isReopening = () => reopening() === s.id;
              const color = () => RUNTIME_COLOR[s.runtimeKind ?? ""] ?? "text-zinc-500";
              return (
                <button
                  onClick={() => {
                    if (isClosed()) { void handleReopen(s); } else { setSelectedId(s.id); }
                  }}
                  disabled={isReopening()}
                  class={`group flex w-full flex-col gap-0.5 border-b theme-border px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === s.id && !isClosed() ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-accent-soft)]"} ${isClosed() ? "opacity-60 hover:opacity-100" : ""}`}
                  title={isClosed() ? "Click to reopen" : undefined}
                >
                  <div class="flex items-center gap-1.5 min-w-0">
                    <Show when={isActive()}>
                      <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400 shadow-[0_0_4px_rgba(52,211,153,0.5)]" />
                    </Show>
                    <Show when={isClosed()}>
                      <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-zinc-600" />
                    </Show>
                    <span class={`truncate font-mono text-[10px] font-semibold ${isClosed() ? "theme-muted" : selectedId() === s.id ? "theme-text" : "theme-muted"}`}>{s.title}</span>
                    <Show when={isReopening()}>
                      <span class="ml-auto font-mono text-[9px] theme-muted shrink-0">opening…</span>
                    </Show>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <Show when={s.runtimeKind}>
                      <span class={`font-mono text-[9px] ${color()}`}>{s.runtimeKind}</span>
                      <span class="theme-muted opacity-40">·</span>
                    </Show>
                    <span class="font-mono text-[9px] theme-muted">{s.messageCount} msgs</span>
                    <span class="theme-muted opacity-40">·</span>
                    <span class="font-mono text-[9px] theme-muted opacity-60">{fmtRelative(s.updatedAt || s.createdAt)}</span>
                  </div>
                </button>
              );
            }}
          </For>
        </div>
      </div>

      {/* Detail pane */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={selected()} fallback={
          <EmptyState icon="◫" title="Select a session" sub="Click any session on the left to inspect it" />
        }>
          {(s) => (
            <div class="space-y-5">
              {/* Header */}
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="font-mono text-sm font-bold theme-text">{s().title}</h2>
                  <p class="mt-0.5 font-mono text-[10px] theme-muted">{s().id}</p>
                </div>
                <div class="flex gap-2 shrink-0">
                  <Show when={activeIds().has(s().id)}>
                    <Badge label="active" color="bg-emerald-500/15 text-emerald-300" />
                  </Show>
                  <Show when={s().closedAt !== null && !activeIds().has(s().id)}>
                    <ActionButton
                      label={reopening() === s().id ? "Opening…" : "Reopen"}
                      variant="ghost"
                      onClick={() => { if (s().closedAt !== null) void handleReopen(s()); }}
                    />
                  </Show>
                </div>
              </div>

              {/* Metadata */}
              <div class="space-y-2 rounded-lg border theme-border theme-surface p-4">
                <FieldRow label="Role">
                  <span class="font-mono text-xs text-zinc-200">{s().activeRole}</span>
                </FieldRow>
                <FieldRow label="Runtime">
                  <span class={`font-mono text-xs ${RUNTIME_COLOR[s().runtimeKind ?? ""] ?? "text-zinc-500"}`}>
                    {s().runtimeKind ?? "—"}
                  </span>
                </FieldRow>
                <FieldRow label="Directory">
                  <span class="break-all font-mono text-[10px] theme-muted">{s().cwd ?? "—"}</span>
                </FieldRow>
                <FieldRow label="Messages">
                  <span class="font-mono text-xs text-zinc-200">{s().messageCount}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] theme-muted">{s().createdAt ? fmtDate(s().createdAt) : "—"}</span>
                </FieldRow>
                <FieldRow label="Updated">
                  <span class="font-mono text-[10px] theme-muted">{s().updatedAt ? fmtDate(s().updatedAt) : "—"}</span>
                </FieldRow>
                <Show when={s().closedAt}>
                  <FieldRow label="Closed">
                    <span class="font-mono text-[10px] theme-muted opacity-60">{fmtDate(s().closedAt!)}</span>
                  </FieldRow>
                </Show>
              </div>

              {/* Context — full CRUD, scoped to this session */}
              <div class="rounded-lg border theme-border theme-surface p-4">
                <SessionContextSection sessionId={s().id} />
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
