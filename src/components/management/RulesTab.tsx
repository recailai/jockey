import { For, Show, createSignal, onMount } from "solid-js";
import { INTERACTIVE_MOTION } from "../types";
import { ruleApi, type AppRule } from "../../lib/tauriApi";

function genId() {
  return `rule_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
}

export function RulesTab() {
  const [rules, setRules] = createSignal<AppRule[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [editName, setEditName] = createSignal("");
  const [editDesc, setEditDesc] = createSignal("");
  const [editContent, setEditContent] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [deletingId, setDeletingId] = createSignal<string | null>(null);
  const [error, setError] = createSignal("");

  const loadRules = async () => {
    setLoading(true);
    try {
      setRules(await ruleApi.list());
    } finally {
      setLoading(false);
    }
  };

  onMount(() => { void loadRules(); });

  const selectedRule = () => rules().find((r) => r.id === selectedId()) ?? null;

  const selectRule = (r: AppRule) => {
    setSelectedId(r.id);
    setEditName(r.name);
    setEditDesc(r.description ?? "");
    setEditContent(r.content);
    setError("");
  };

  const newRule = () => {
    const id = genId();
    setSelectedId(id);
    setEditName("");
    setEditDesc("");
    setEditContent("");
    setError("");
  };

  const handleSave = async () => {
    const id = selectedId();
    if (!id || saving()) return;
    if (!editName().trim()) { setError("Name required"); return; }
    setSaving(true);
    setError("");
    try {
      await ruleApi.upsert(id, editName().trim(), editContent(), editDesc().trim() || null);
      await loadRules();
      setSelectedId(id);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await ruleApi.remove(id);
      if (selectedId() === id) setSelectedId(null);
      setDeletingId(null);
      await loadRules();
    } catch (e) {
      setError(String(e));
    }
  };

  const isNew = () => !!selectedId() && !rules().some((r) => r.id === selectedId());

  return (
    <div class="flex h-full">
      <div class="flex w-56 shrink-0 flex-col border-r theme-border">
        <div class="flex items-center justify-between px-3 py-2 border-b theme-border">
          <span class="text-[10px] uppercase tracking-widest theme-muted font-bold">Rules</span>
          <button
            class={`flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-bold border theme-border theme-muted hover:text-primary hover:border-[var(--ui-border-strong)] ${INTERACTIVE_MOTION}`}
            onClick={newRule}
          >
            <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
            New
          </button>
        </div>
        <div class="flex-1 overflow-auto">
          <Show when={loading()}>
            <div class="px-3 py-2 text-[10px] theme-muted">Loading...</div>
          </Show>
          <For each={rules()}>{(r) => (
            <div
              onClick={() => selectRule(r)}
              class={`group flex cursor-pointer items-center justify-between px-3 py-2 border-b border-white/5 hover:bg-[var(--ui-accent-soft)] transition-colors ${selectedId() === r.id ? "bg-[var(--ui-accent-soft)] text-primary" : "theme-muted"}`}
            >
              <div class="min-w-0 flex-1">
                <div class="truncate text-[11px] font-medium theme-text">{r.name}</div>
                <Show when={r.description}>
                  <div class="truncate text-[9.5px] theme-muted">{r.description}</div>
                </Show>
              </div>
              <Show when={deletingId() === r.id} fallback={
                <button
                  onClick={(e) => { e.stopPropagation(); setDeletingId(r.id); }}
                  class="opacity-0 group-hover:opacity-60 hover:!opacity-100 hover:text-rose-400 transition-all ml-1"
                >
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
                </button>
              }>
                <div class="flex items-center gap-1">
                  <button onClick={(e) => { e.stopPropagation(); void handleDelete(r.id); }} class="text-[9px] text-rose-400 hover:text-rose-300 font-bold">Del</button>
                  <button onClick={(e) => { e.stopPropagation(); setDeletingId(null); }} class="text-[9px] theme-muted hover:text-primary font-bold">✕</button>
                </div>
              </Show>
            </div>
          )}</For>
        </div>
      </div>

      <div class="flex flex-1 flex-col overflow-hidden">
        <Show when={selectedId()} fallback={
          <div class="flex flex-1 items-center justify-center theme-muted text-[12px]">
            Select a rule or create a new one
          </div>
        }>
          <div class="flex items-center justify-between border-b theme-border px-4 py-2">
            <span class="text-[11px] font-bold theme-text truncate">
              {isNew() ? "New Rule" : selectedRule()?.name ?? ""}
            </span>
            <div class="flex items-center gap-2">
              <Show when={error()}>
                <span class="management-error-text">{error()}</span>
              </Show>
              <button
                onClick={() => void handleSave()}
                disabled={saving()}
                class={`management-primary-button ${INTERACTIVE_MOTION}`}
              >
                {saving() ? "Saving..." : "Save"}
              </button>
            </div>
          </div>
          <div class="flex-1 overflow-auto px-4 py-3 space-y-3">
            <div>
              <label class="mb-1 block text-[10px] uppercase tracking-wider theme-muted font-bold">Name</label>
              <input
                class="w-full rounded-md border theme-border bg-[var(--ui-panel-2)] px-2.5 py-1.5 text-[12px] theme-text outline-none focus:border-indigo-500/60 focus:ring-1 focus:ring-indigo-500/30"
                value={editName()}
                onInput={(e) => setEditName(e.currentTarget.value)}
                placeholder="rule-name"
              />
            </div>
            <div>
              <label class="mb-1 block text-[10px] uppercase tracking-wider theme-muted font-bold">Description</label>
              <input
                class="w-full rounded-md border theme-border bg-[var(--ui-panel-2)] px-2.5 py-1.5 text-[12px] theme-text outline-none focus:border-indigo-500/60 focus:ring-1 focus:ring-indigo-500/30"
                value={editDesc()}
                onInput={(e) => setEditDesc(e.currentTarget.value)}
                placeholder="Short description (optional)"
              />
            </div>
            <div class="flex-1">
              <label class="mb-1 block text-[10px] uppercase tracking-wider theme-muted font-bold">Content</label>
              <textarea
                class="w-full rounded-md border theme-border bg-[var(--ui-panel-2)] px-2.5 py-1.5 text-[12px] theme-text font-mono outline-none focus:border-indigo-500/60 focus:ring-1 focus:ring-indigo-500/30 resize-none"
                rows={16}
                value={editContent()}
                onInput={(e) => setEditContent(e.currentTarget.value)}
                placeholder="Rule content (markdown supported)"
              />
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
}
