import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { Alert, Button, DetailPane, FormField, Input, MasterDetailView, MasterItem, MasterPane, Textarea } from "../ui";
import { ruleApi, type AppRule } from "../../lib/tauriApi";

function genId() {
  return `rule_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
}

export function RulesTab() {
  const [rules, setRules] = createSignal<AppRule[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [search, setSearch] = createSignal("");
  const [editName, setEditName] = createSignal("");
  const [editDesc, setEditDesc] = createSignal("");
  const [editContent, setEditContent] = createSignal("");
  const [saving, setSaving] = createSignal(false);
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

  const filteredRules = createMemo(() => {
    const q = search().toLowerCase().trim();
    if (!q) return rules();
    return rules().filter((r) =>
      r.name.toLowerCase().includes(q) || (r.description ?? "").toLowerCase().includes(q),
    );
  });

  const selectedRule = () => rules().find((r) => r.id === selectedId()) ?? null;
  const isNew = () => !!selectedId() && !rules().some((r) => r.id === selectedId());

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
    if (!editName().trim()) { setError("Rule name is required"); return; }
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
      await loadRules();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <MasterDetailView>
      <MasterPane
        title="Rules"
        action={{ label: "New", onClick: newRule }}
        search={{
          value: search(),
          onInput: setSearch,
          placeholder: "Filter rules…",
        }}
      >
        <Show when={loading()}>
          <div class="px-3 py-3 text-xs theme-muted">Loading rules…</div>
        </Show>
        <Show when={!loading() && filteredRules().length === 0}>
          <div class="px-3 py-6 text-center text-xs theme-muted">
            {search() ? "No matching rules" : "No rules defined yet"}
          </div>
        </Show>
        <For each={filteredRules()}>
          {(r) => (
            <MasterItem
              title={r.name}
              subtitle={r.description ?? undefined}
              active={selectedId() === r.id}
              onClick={() => selectRule(r)}
              onDelete={() => void handleDelete(r.id)}
            />
          )}
        </For>
      </MasterPane>

      <DetailPane
        title={selectedId() ? (isNew() ? "New Rule" : selectedRule()?.name ?? "Rule") : undefined}
        subtitle={selectedId() ? (isNew() ? "Configure new rule prompt" : (selectedRule()?.description ?? undefined)) : undefined}
        empty={!selectedId()}
        emptyFallback={
          <div class="flex flex-1 items-center justify-center text-xs theme-muted">
            Select a rule from the left or create a new one
          </div>
        }
        actions={
          <Show when={selectedId()}>
            <Button
              variant="default"
              size="sm"
              disabled={saving()}
              onClick={() => void handleSave()}
            >
              {saving() ? "Saving…" : "Save Rule"}
            </Button>
          </Show>
        }
      >
        <div class="max-w-2xl space-y-4">
          <Show when={error()}>
            <Alert tone="danger" onClose={() => setError("")}>
              {error()}
            </Alert>
          </Show>

          <FormField label="Rule Name" required>
            <Input
              value={editName()}
              onInput={(e) => setEditName(e.currentTarget.value)}
              placeholder="e.g. strict-typescript, commit-standards"
            />
          </FormField>

          <FormField label="Description">
            <Input
              value={editDesc()}
              onInput={(e) => setEditDesc(e.currentTarget.value)}
              placeholder="Short summary of when this rule applies"
            />
          </FormField>

          <FormField label="Rule Content (Markdown)">
            <Textarea
              monospace
              rows={16}
              value={editContent()}
              onInput={(e) => setEditContent(e.currentTarget.value)}
              placeholder="Enter system prompt instructions, style guides, or constraints…"
            />
          </FormField>
        </div>
      </DetailPane>
    </MasterDetailView>
  );
}
