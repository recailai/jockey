import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import type { Role } from "../types";
import { RUNTIME_COLOR } from "../types";
import { EmptyState, FieldRow, TextInput, InlineSelect, PanelSection, ActionButton } from "./primitives";
import type { Workflow, WorkflowStep } from "./primitives";
import { fmtDate, fmtRelative } from "./primitives";
import { workflowApi } from "../../lib/tauriApi";

export function WorkflowsTab(props: { roles: Role[] }) {
  const [workflows, setWorkflows] = createSignal<Workflow[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [wfName, setWfName] = createSignal("");
  const [wfDesc, setWfDesc] = createSignal("");
  const [wfSteps, setWfSteps] = createSignal<WorkflowStep[]>([{ roleName: "", prompt: "", order: 0 }]);

  const load = async () => {
    setLoading(true);
    try {
      const raw = await workflowApi.list<Workflow[]>();
      setWorkflows(raw);
      if (raw.length > 0 && !selectedId()) setSelectedId(raw[0].id);
    } catch { setWorkflows([]); }
    setLoading(false);
  };

  onMount(() => void load());

  const selected = createMemo(() => workflows().find((w) => w.id === selectedId()) ?? null);

  const addStep = () =>
    setWfSteps((s) => [...s, { roleName: "", prompt: "", order: s.length }]);

  const removeStep = (idx: number) =>
    setWfSteps((s) => s.filter((_, i) => i !== idx).map((step, i) => ({ ...step, order: i })));

  const updateStep = (idx: number, patch: Partial<WorkflowStep>) =>
    setWfSteps((s) => s.map((step, i) => i === idx ? { ...step, ...patch } : step));

  const handleCreate = async () => {
    const name = wfName().trim();
    if (!name) return;
    try {
      await workflowApi.create(name, wfDesc().trim(), wfSteps());
      setCreating(false);
      setWfName(""); setWfDesc("");
      setWfSteps([{ roleName: "", prompt: "", order: 0 }]);
      await load();
    } catch { /* TODO: error toast */ }
  };

  const handleDelete = async (id: string) => {
    try {
      await workflowApi.remove(id);
      if (selectedId() === id) setSelectedId(null);
      await load();
    } catch { /* ignore */ }
  };

  const roleOptions = createMemo(() =>
    [{ value: "", label: "— Select role —" }, ...props.roles.map((r) => ({ value: r.roleName, label: r.roleName }))]
  );

  return (
    <div class="flex h-full">
      {/* List pane */}
      <div class="flex w-56 shrink-0 flex-col border-r theme-border">
        <div class="p-3">
          <ActionButton
            label="+ New Automation"
            variant="ghost"
            class="w-full justify-center text-center"
            onClick={() => { setCreating(true); setSelectedId(null); }}
          />
        </div>
        <div class="flex-1 overflow-y-auto space-y-0.5 py-1">
          <Show when={loading()}>
            <p class="p-4 font-mono text-[10px] theme-muted">Loading…</p>
          </Show>
          <Show when={!loading() && workflows().length === 0}>
            <EmptyState icon="⬡" title="No automations" sub="Create your first automation" />
          </Show>
          <For each={workflows()}>
            {(wf) => (
              <button
                onClick={() => { setSelectedId(wf.id); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 rounded-lg mx-1.5 px-2.5 py-2 text-left transition-colors duration-100 ${selectedId() === wf.id ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"}`}
              >
                <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === wf.id ? "theme-text" : "theme-text"}`}>{wf.name}</span>
                <div class="flex items-center gap-1.5">
                  <span class="font-mono text-[9px] theme-muted">{wf.steps?.length ?? 0} steps</span>
                  <span class="theme-muted opacity-40">·</span>
                  <span class="font-mono text-[9px] theme-muted">{fmtRelative(wf.updatedAt || wf.createdAt)}</span>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>

      {/* Detail / create pane */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={creating()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold theme-text uppercase tracking-widest">New Automation</h3>
            <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
              <FieldRow label="Name">
                <TextInput value={wfName()} onInput={setWfName} placeholder="e.g. code-review" />
              </FieldRow>
              <FieldRow label="Desc">
                <TextInput value={wfDesc()} onInput={setWfDesc} placeholder="Optional description" />
              </FieldRow>
            </div>

            <PanelSection title="Steps" action={{ label: "+ Step", onClick: addStep }}>
              <div class="space-y-2">
                <For each={wfSteps()}>
                  {(step, i) => (
                    <div class="rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-3 space-y-2">
                      <div class="flex items-center justify-between">
                        <span class="font-mono text-[9px] theme-muted uppercase tracking-widest">Step {i() + 1}</span>
                        <Show when={wfSteps().length > 1}>
                          <button
                            onClick={() => removeStep(i())}
                            class="font-mono text-[9px] theme-muted hover:text-rose-400 transition-colors"
                          >
                            remove
                          </button>
                        </Show>
                      </div>
                      <InlineSelect
                        value={step.roleName}
                        options={roleOptions()}
                        onChange={(v) => updateStep(i(), { roleName: v })}
                      />
                      <TextInput
                        value={step.prompt}
                        onInput={(v) => updateStep(i(), { prompt: v })}
                        placeholder="Prompt for this step…"
                        multiline
                        rows={2}
                      />
                    </div>
                  )}
                </For>
              </div>
            </PanelSection>

            <div class="flex gap-2">
              <ActionButton label="Create" variant="primary" onClick={() => void handleCreate()} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>
          </div>
        </Show>

        <Show when={!creating() && selected()}>
          {(wf) => (
            <div class="space-y-5">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="font-mono text-sm font-bold theme-text">{wf().name}</h2>
                  <Show when={wf().description}>
                    <p class="mt-1 text-[10px] theme-muted">{wf().description}</p>
                  </Show>
                </div>
                <ActionButton label="Delete" variant="danger" onClick={() => void handleDelete(wf().id)} />
              </div>

              <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
                <FieldRow label="ID">
                  <span class="font-mono text-[10px] theme-muted">{wf().id}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] theme-muted">{fmtDate(wf().createdAt)}</span>
                </FieldRow>
              </div>

              <PanelSection title={`Steps (${wf().steps?.length ?? 0})`}>
                <div class="space-y-2">
                  <Show when={(wf().steps?.length ?? 0) === 0}>
                    <p class="font-mono text-[10px] theme-muted">No steps defined.</p>
                  </Show>
                  <For each={wf().steps ?? []}>
                    {(step, i) => (
                      <div class="rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-3">
                        <div class="mb-1.5 flex items-center gap-2">
                          <span class="font-mono text-[9px] theme-muted uppercase tracking-widest">Step {i() + 1}</span>
                          <span class={`font-mono text-[9px] font-semibold ${RUNTIME_COLOR[props.roles.find((r) => r.roleName === step.roleName)?.runtimeKind ?? ""] ?? "theme-muted"}`}>
                            {step.roleName || "—"}
                          </span>
                        </div>
                        <p class="text-[10px] theme-muted leading-relaxed">{step.prompt}</p>
                      </div>
                    )}
                  </For>
                </div>
              </PanelSection>
            </div>
          )}
        </Show>

        <Show when={!creating() && !selected()}>
          <EmptyState icon="⬡" title="Select an automation" sub="Or create a new one" />
        </Show>
      </div>
    </div>
  );
}
