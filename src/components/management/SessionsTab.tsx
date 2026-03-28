import { For, Show, createEffect, createMemo, createSignal, onMount } from "solid-js";
import type { AppSession } from "../types";
import { RUNTIME_COLOR } from "../types";
import { Badge, EmptyState, FieldRow, ActionButton } from "./primitives";
import type { ContextEntry, StoredSession } from "./primitives";
import { fmtDate, fmtRelative } from "./primitives";
import { appSessionApi, commandApi } from "../../lib/tauriApi";

export function SessionsTab(props: {
  activeSessions: AppSession[];
  onRestoreSession?: (id: string, title: string, activeRole: string, runtimeKind: string | null, cwd: string | null) => void;
}) {
  const [storedSessions, setStoredSessions] = createSignal<StoredSession[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [reopening, setReopening] = createSignal<string | null>(null);
  const [search, setSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [contextEntries, setContextEntries] = createSignal<ContextEntry[]>([]);
  const [contextLoading, setContextLoading] = createSignal(false);
  let contextReqSeq = 0;

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
      const activeMapped = active.map((r) => mapRaw(r, null));
      const closedMapped = closed.map((r) => mapRaw(r, r.closedAt ?? null));
      setStoredSessions([...activeMapped, ...closedMapped]);
    } catch { /* ignore */ }
    setLoading(false);
  };

  onMount(() => void load());

  createEffect(() => {
    const sid = selectedId();
    const reqSeq = ++contextReqSeq;
    if (!sid) { setContextEntries([]); setContextLoading(false); return; }
    setContextLoading(true);
    commandApi.apply<{ entries?: ContextEntry[] }>("/app_context list", sid).then((res) => {
      if (reqSeq !== contextReqSeq) return;
      setContextEntries(res.payload.entries ?? []);
    }).catch(() => {
      if (reqSeq !== contextReqSeq) return;
      setContextEntries([]);
    }).finally(() => {
      if (reqSeq !== contextReqSeq) return;
      setContextLoading(false);
    });
  });

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
            class="h-7 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2.5 font-mono text-[10px] text-zinc-300 placeholder:text-zinc-700 focus:border-zinc-600 focus:outline-none"
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={loading()}>
            <p class="p-4 font-mono text-[10px] text-zinc-700">Loading…</p>
          </Show>
          <Show when={!loading() && filtered().length === 0}>
            <p class="p-4 font-mono text-[10px] text-zinc-700">No sessions found.</p>
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
                  class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === s.id && !isClosed() ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"} ${isClosed() ? "opacity-60 hover:opacity-100" : ""}`}
                  title={isClosed() ? "Click to reopen" : undefined}
                >
                  <div class="flex items-center gap-1.5 min-w-0">
                    <Show when={isActive()}>
                      <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400 shadow-[0_0_4px_rgba(52,211,153,0.5)]" />
                    </Show>
                    <Show when={isClosed()}>
                      <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-zinc-600" />
                    </Show>
                    <span class={`truncate font-mono text-[10px] font-semibold ${isClosed() ? "text-zinc-500" : selectedId() === s.id ? "text-zinc-100" : "text-zinc-300"}`}>{s.title}</span>
                    <Show when={isReopening()}>
                      <span class="ml-auto font-mono text-[9px] text-zinc-600 shrink-0">opening…</span>
                    </Show>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <Show when={s.runtimeKind}>
                      <span class={`font-mono text-[9px] ${color()}`}>{s.runtimeKind}</span>
                      <span class="text-zinc-700">·</span>
                    </Show>
                    <span class="font-mono text-[9px] text-zinc-600">{s.messageCount} msgs</span>
                    <span class="text-zinc-700">·</span>
                    <span class="font-mono text-[9px] text-zinc-700">{fmtRelative(s.updatedAt || s.createdAt)}</span>
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
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{s().title}</h2>
                  <p class="mt-0.5 font-mono text-[10px] text-zinc-600">{s().id}</p>
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

              <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                <FieldRow label="Role">
                  <span class="font-mono text-xs text-zinc-200">{s().activeRole}</span>
                </FieldRow>
                <FieldRow label="Runtime">
                  <span class={`font-mono text-xs ${RUNTIME_COLOR[s().runtimeKind ?? ""] ?? "text-zinc-500"}`}>
                    {s().runtimeKind ?? "—"}
                  </span>
                </FieldRow>
                <FieldRow label="Directory">
                  <span class="break-all font-mono text-[10px] text-zinc-400">{s().cwd ?? "—"}</span>
                </FieldRow>
                <FieldRow label="Messages">
                  <span class="font-mono text-xs text-zinc-200">{s().messageCount}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] text-zinc-500">{s().createdAt ? fmtDate(s().createdAt) : "—"}</span>
                </FieldRow>
                <FieldRow label="Updated">
                  <span class="font-mono text-[10px] text-zinc-500">{s().updatedAt ? fmtDate(s().updatedAt) : "—"}</span>
                </FieldRow>
                <Show when={s().closedAt}>
                  <FieldRow label="Closed">
                    <span class="font-mono text-[10px] text-zinc-600">{fmtDate(s().closedAt!)}</span>
                  </FieldRow>
                </Show>
              </div>

              {/* Context entries */}
              <div>
                <div class="mb-2 flex items-center justify-between">
                  <span class="font-mono text-[10px] font-semibold uppercase tracking-widest text-zinc-500">Context</span>
                  <Show when={contextLoading()}>
                    <span class="font-mono text-[9px] text-zinc-700">loading…</span>
                  </Show>
                </div>
                <Show when={!contextLoading() && contextEntries().length === 0}>
                  <p class="font-mono text-[10px] text-zinc-700 italic">no context entries</p>
                </Show>
                <Show when={contextEntries().length > 0}>
                  <table class="w-full border-collapse font-mono text-[10px]">
                    <thead>
                      <tr class="border-b border-white/[0.04] text-left text-zinc-600">
                        <th class="py-1.5 pr-4 font-semibold uppercase tracking-widest">scope</th>
                        <th class="py-1.5 pr-4 font-semibold uppercase tracking-widest">key</th>
                        <th class="py-1.5 font-semibold uppercase tracking-widest">value</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={contextEntries()}>
                        {(e) => {
                          const formatted = () => {
                            try { return JSON.stringify(JSON.parse(e.value), null, 2); }
                            catch { return e.value; }
                          };
                          return (
                            <tr class="border-b border-white/[0.03]">
                              <td class="py-2 pr-4 align-top text-zinc-500">{e.scope}</td>
                              <td class="py-2 pr-4 align-top text-amber-400/80">{e.key}</td>
                              <td class="py-2 align-top text-zinc-300">
                                <pre class="whitespace-pre-wrap break-all font-mono text-[9px] leading-relaxed">{formatted()}</pre>
                              </td>
                            </tr>
                          );
                        }}
                      </For>
                    </tbody>
                  </table>
                </Show>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
