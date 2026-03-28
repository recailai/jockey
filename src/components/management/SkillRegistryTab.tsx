import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { AppSkill } from "../types";
import { INTERACTIVE_MOTION } from "../types";
import { EmptyState, FieldRow, TextInput, PanelSection, ActionButton, fmtDate } from "./primitives";
import { skillApi } from "../../lib/tauriApi";

export function SkillRegistryTab(props: {
  skills: AppSkill[];
  refreshSkills: () => Promise<void>;
}) {
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const [search, setSearch] = createSignal("");

  // Form
  const [fName, setFName] = createSignal("");
  const [fDesc, setFDesc] = createSignal("");
  const [fContent, setFContent] = createSignal("");

  createEffect(() => {
    if (props.skills.length > 0 && !selectedId()) {
      setSelectedId(props.skills[0].id);
    }
  });

  const filtered = createMemo(() => {
    const q = search().toLowerCase();
    return props.skills.filter((s) =>
      !q || s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
    );
  });

  const selected = createMemo(() =>
    filtered().find((s) => s.id === selectedId()) ?? null,
  );

  const openCreate = () => {
    setFName(""); setFDesc(""); setFContent("");
    setCreating(true); setEditing(false); setSelectedId(null);
  };

  const openEdit = (s: AppSkill) => {
    setFName(s.name); setFDesc(s.description); setFContent(s.content);
    setEditing(true); setCreating(false);
  };

  const handleSave = async () => {
    const name = fName().trim();
    if (!name) return;
    const payload = { name, description: fDesc().trim(), content: fContent().trim() };
    if (editing() && selectedId()) {
      try {
        await skillApi.upsert({ id: selectedId()!, ...payload });
      } catch { /* ignore */ }
    } else {
      try {
        await skillApi.upsert(payload);
      } catch { /* ignore */ }
    }
    await props.refreshSkills();
    setCreating(false); setEditing(false);
  };

  const handleDelete = async (id: string) => {
    try {
      await skillApi.remove(id);
      if (selectedId() === id) setSelectedId(null);
      await props.refreshSkills();
    } catch { /* ignore */ }
  };

  return (
    <div class="flex h-full">
      {/* List */}
      <div class="flex w-64 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="flex gap-2 border-b border-white/[0.04] p-3">
          <input
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
            placeholder="Filter skills…"
            class="h-7 flex-1 rounded-md border border-zinc-800 bg-zinc-950/60 px-2.5 font-mono text-[10px] text-zinc-300 placeholder:text-zinc-700 focus:border-zinc-600 focus:outline-none"
          />
          <button
            onClick={openCreate}
            class={`flex h-7 w-7 items-center justify-center rounded-md border border-zinc-800 text-zinc-500 hover:border-zinc-600 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
            title="New skill"
          >
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" /></svg>
          </button>
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={filtered().length === 0}>
            <EmptyState icon="⚡" title="No skills yet" sub="Skills extend what agents can invoke" />
          </Show>
          <For each={filtered()}>
            {(skill) => (
              <button
                onClick={() => { setSelectedId(skill.id); setCreating(false); setEditing(false); }}
                class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === skill.id ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"}`}
              >
                <span class={`font-mono text-[10px] font-semibold ${selectedId() === skill.id ? "text-zinc-100" : "text-zinc-300"}`}>{skill.name}</span>
                <span class="font-mono text-[9px] text-zinc-600 truncate">{skill.description || "No description"}</span>
              </button>
            )}
          </For>
        </div>
      </div>

      {/* Detail */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={creating() || editing()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold text-zinc-300 uppercase tracking-widest">
              {creating() ? "New Skill" : `Edit: ${selected()?.name ?? ""}`}
            </h3>
            <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
              <FieldRow label="Name">
                <TextInput value={fName()} onInput={setFName} placeholder="skill-name" monospace />
              </FieldRow>
              <FieldRow label="Desc">
                <TextInput value={fDesc()} onInput={setFDesc} placeholder="What this skill does…" />
              </FieldRow>
              <FieldRow label="Content">
                <TextInput
                  value={fContent()}
                  onInput={setFContent}
                  placeholder="Skill prompt / instructions…"
                  multiline
                  rows={8}
                />
              </FieldRow>
            </div>
            <div class="flex gap-2">
              <ActionButton label="Save" variant="primary" onClick={() => void handleSave()} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => { setCreating(false); setEditing(false); }} />
            </div>
          </div>
        </Show>

        <Show when={!creating() && !editing() && selected()}>
          {(skill) => (
            <div class="space-y-5">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{skill().name}</h2>
                  <Show when={skill().description}>
                    <p class="mt-1 text-[10px] text-zinc-500">{skill().description}</p>
                  </Show>
                </div>
                <div class="flex gap-2 shrink-0">
                  <ActionButton label="Edit" variant="ghost" onClick={() => openEdit(skill())} />
                  <ActionButton label="Delete" variant="danger" onClick={() => void handleDelete(skill().id)} />
                </div>
              </div>

              <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                <FieldRow label="ID">
                  <span class="font-mono text-[10px] text-zinc-600">{skill().id}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] text-zinc-500">{fmtDate(skill().createdAt)}</span>
                </FieldRow>
                <FieldRow label="Updated">
                  <span class="font-mono text-[10px] text-zinc-500">{fmtDate(skill().updatedAt)}</span>
                </FieldRow>
              </div>

              <PanelSection title="Content">
                <pre class="whitespace-pre-wrap rounded-lg border border-white/[0.03] bg-zinc-950/60 p-3 font-mono text-[10px] leading-relaxed text-zinc-300">
                  {skill().content || <span class="text-zinc-700">empty</span>}
                </pre>
              </PanelSection>

              <div class="rounded-lg border border-zinc-800/40 bg-zinc-900/20 p-3">
                <p class="font-mono text-[10px] text-zinc-600 leading-relaxed">
                  Invoke with <code class="text-teal-400">/{skill().name}</code> in the chat input or reference as <code class="text-teal-400">@{skill().name}</code> in prompts.
                </p>
              </div>
            </div>
          )}
        </Show>

        <Show when={!creating() && !editing() && !selected()}>
          <EmptyState icon="⚡" title="Select a skill" sub="Or create a new one" />
        </Show>
      </div>
    </div>
  );
}
