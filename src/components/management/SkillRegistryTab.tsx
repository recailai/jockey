import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { Alert, Badge, Button, DetailPane, FormField, Input, MasterDetailView, MasterItem, MasterPane, Textarea } from "../ui";
import { fmtDate } from "../../lib/formatHelpers";
import { skillApi } from "../../lib/tauriApi";
import type { AppSkill } from "../types";

export function SkillRegistryTab(props: {
  skills: AppSkill[];
  refreshSkills: () => Promise<void>;
}) {
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const [search, setSearch] = createSignal("");
  const [error, setError] = createSignal("");

  // Form fields
  const [fName, setFName] = createSignal("");
  const [fDesc, setFDesc] = createSignal("");
  const [fContent, setFContent] = createSignal("");
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    if (props.skills.length > 0 && !selectedId() && !creating()) {
      setSelectedId(props.skills[0].id);
    }
  });

  const filtered = createMemo(() => {
    const q = search().toLowerCase().trim();
    return props.skills.filter((s) =>
      !q || s.name.toLowerCase().includes(q) || (s.description ?? "").toLowerCase().includes(q),
    );
  });

  const selected = createMemo(() =>
    filtered().find((s) => s.id === selectedId()) ?? null,
  );

  const openCreate = () => {
    setFName("");
    setFDesc("");
    setFContent("");
    setError("");
    setCreating(true);
    setEditing(false);
    setSelectedId(null);
  };

  const openEdit = (s: AppSkill) => {
    setFName(s.name);
    setFDesc(s.description ?? "");
    setFContent(s.content);
    setError("");
    setEditing(true);
    setCreating(false);
  };

  const handleSave = async () => {
    const name = fName().trim();
    if (!name) {
      setError("Skill name is required");
      return;
    }
    setSaving(true);
    setError("");
    const payload = { name, description: fDesc().trim(), content: fContent().trim() };
    try {
      if (editing() && selectedId()) {
        await skillApi.upsert({ id: selectedId()!, ...payload });
      } else {
        await skillApi.upsert(payload);
      }
      await props.refreshSkills();
      setCreating(false);
      setEditing(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await skillApi.remove(id);
      if (selectedId() === id) setSelectedId(null);
      await props.refreshSkills();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <MasterDetailView>
      <MasterPane
        title="Skills"
        action={{ label: "New", onClick: openCreate }}
        search={{
          value: search(),
          onInput: setSearch,
          placeholder: "Filter skills…",
        }}
      >
        <Show when={filtered().length === 0}>
          <div class="px-3 py-6 text-center text-xs theme-muted">
            {search() ? "No matching skills" : "No skills registered yet"}
          </div>
        </Show>
        <For each={filtered()}>
          {(skill) => (
            <MasterItem
              title={skill.name}
              subtitle={skill.description || "No description"}
              active={selectedId() === skill.id && !creating()}
              onClick={() => {
                setSelectedId(skill.id);
                setCreating(false);
                setEditing(false);
                setError("");
              }}
              onDelete={() => void handleDelete(skill.id)}
            />
          )}
        </For>
      </MasterPane>

      <DetailPane
        title={
          creating()
            ? "New Skill"
            : editing()
            ? `Edit: ${selected()?.name ?? "Skill"}`
            : selected()?.name ?? undefined
        }
        subtitle={
          creating()
            ? "Create a reusable prompt/tool extension"
            : editing()
            ? "Update skill metadata and instructions"
            : selected()?.description ?? undefined
        }
        empty={!creating() && !editing() && !selected()}
        emptyFallback={
          <div class="flex flex-1 items-center justify-center text-xs theme-muted">
            Select a skill or create a new one
          </div>
        }
        actions={
          <Show
            when={creating() || editing()}
            fallback={
              <Show when={selected()}>
                {(skill) => (
                  <div class="flex items-center gap-2">
                    <Button variant="outline" size="sm" onClick={() => openEdit(skill())}>
                      Edit
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => void handleDelete(skill().id)}
                    >
                      Delete
                    </Button>
                  </div>
                )}
              </Show>
            }
          >
            <div class="flex items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                onClick={() => {
                  setCreating(false);
                  setEditing(false);
                  setError("");
                }}
              >
                Cancel
              </Button>
              <Button
                variant="default"
                size="sm"
                disabled={saving()}
                onClick={() => void handleSave()}
              >
                {saving() ? "Saving…" : "Save Skill"}
              </Button>
            </div>
          </Show>
        }
      >
        <Show when={error()}>
          <Alert tone="danger" onClose={() => setError("")} class="mb-4">
            {error()}
          </Alert>
        </Show>

        <Show
          when={creating() || editing()}
          fallback={
            <Show when={selected()}>
              {(skill) => (
                <div class="max-w-2xl space-y-5">
                  <div class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-surface-muted)] p-3 space-y-2 text-xs">
                    <div class="flex items-center justify-between">
                      <span class="theme-muted">Skill ID</span>
                      <span class="font-mono text-[11px] theme-text">{skill().id}</span>
                    </div>
                    <div class="flex items-center justify-between">
                      <span class="theme-muted">Created</span>
                      <span class="font-mono text-[11px] theme-text">{fmtDate(skill().createdAt)}</span>
                    </div>
                    <div class="flex items-center justify-between">
                      <span class="theme-muted">Updated</span>
                      <span class="font-mono text-[11px] theme-text">{fmtDate(skill().updatedAt)}</span>
                    </div>
                  </div>

                  <div class="space-y-2">
                    <div class="flex items-center justify-between">
                      <span class="text-xs font-semibold uppercase tracking-wider theme-muted">Instructions / Prompt</span>
                      <Badge tone="info" variant="subtle" label="Active" />
                    </div>
                    <pre class="whitespace-pre-wrap rounded-lg border border-[var(--ui-border)] bg-[var(--ui-card-bg)] p-3 font-mono text-xs leading-relaxed theme-text max-h-[380px] overflow-y-auto">
                      {skill().content || <span class="theme-muted italic">No instructions defined</span>}
                    </pre>
                  </div>

                  <div class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-surface-muted)] p-3 text-xs theme-muted">
                    Invoke with <code class="font-mono font-semibold text-[var(--ui-text)]">/{skill().name}</code> in chat or reference as <code class="font-mono font-semibold text-[var(--ui-text)]">@{skill().name}</code> in instructions.
                  </div>
                </div>
              )}
            </Show>
          }
        >
          <div class="max-w-2xl space-y-4">
            <FormField label="Skill Name" required hint="Used for /slash invocation">
              <Input
                monospace
                value={fName()}
                onInput={(e) => setFName(e.currentTarget.value)}
                placeholder="e.g. explain-code, unit-test-gen"
              />
            </FormField>

            <FormField label="Description">
              <Input
                value={fDesc()}
                onInput={(e) => setFDesc(e.currentTarget.value)}
                placeholder="Brief summary of what this skill does"
              />
            </FormField>

            <FormField label="Instructions & Content" required>
              <Textarea
                monospace
                rows={12}
                value={fContent()}
                onInput={(e) => setFContent(e.currentTarget.value)}
                placeholder="Skill instructions, system prompts, or guidelines to inject…"
              />
            </FormField>
          </div>
        </Show>
      </DetailPane>
    </MasterDetailView>
  );
}
