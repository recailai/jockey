import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, Role, RoleUpsertInput, AcpConfigOption } from "../types";
import { RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "../types";
import { EmptyState, FieldRow, TextInput, InlineSelect, ActionButton } from "./primitives";
import { assistantApi, configApi, roleApi } from "../../lib/tauriApi";

export function RolesTab(props: {
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  refreshRoles: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  pushMessage: (role: string, text: string) => void;
  initialRoleName?: string;
}) {
  const UNION_ROLE = "UnionAIAssistant";
  const userRoles = createMemo(() => props.roles().filter((r) => r.roleName !== UNION_ROLE));

  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [deletingId, setDeletingId] = createSignal<string | null>(null);

  const openCreate = () => {
    const defaultRuntime = "gemini-cli";
    setCName("Developer");
    setCRuntime(defaultRuntime);
    setCPrompt("You are a senior developer. Implement the solution step by step.");
    setCModel("");
    setCMode("");
    setCAutoApprove(true);
    setCConfigOpts([]);
    setCConfigSel({});
    setCreating(true);
    setSelectedId(null);
    setDeletingId(null);
    void props.fetchConfigOptions(defaultRuntime).then(setCConfigOpts);
  };

  // Create form
  const [cName, setCName] = createSignal("Developer");
  const [cRuntime, setCRuntime] = createSignal("gemini-cli");
  const [cPrompt, setCPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
  const [cModel, setCModel] = createSignal("");
  const [cMode, setCMode] = createSignal("");
  const [cAutoApprove, setCAutoApprove] = createSignal(true);
  const [cConfigOpts, setCConfigOpts] = createSignal<AcpConfigOption[]>([]);
  const [cConfigSel, setCConfigSel] = createSignal<Record<string, string>>({});

  // Edit form
  const [ePrompt, setEPrompt] = createSignal("");
  const [eModel, setEModel] = createSignal("");
  const [eMode, setEMode] = createSignal("");
  const [eAutoApprove, setEAutoApprove] = createSignal(true);
  const [eMcpJson, setEMcpJson] = createSignal("[]");
  const [eCfgJson, setECfgJson] = createSignal("{}");

  const editingRole = createMemo(() =>
    selectedId() ? userRoles().find((r) => r.id === selectedId()) ?? null : null,
  );

  const openEdit = (role: Role) => {
    setCreating(false);
    setSelectedId(role.id);
    setEPrompt(role.systemPrompt ?? "");
    setEModel(role.model ?? "");
    setEMode(role.mode ?? "");
    setEAutoApprove(role.autoApprove);
    setEMcpJson(role.mcpServersJson || "[]");
    setECfgJson(role.configOptionsJson || "{}");
    const appSessionId = props.activeSession()?.id ?? "";
    const cached = assistantApi.listDiscoveredConfig(role.runtimeKind, role.roleName, appSessionId);
    void cached.then((raw) => {
      props.patchActiveSession({ discoveredConfigOptions: configApi.asOptions(raw) });
    });
    void assistantApi.prewarmRoleConfig(role.runtimeKind, role.roleName, appSessionId).then((raw) => {
      props.patchActiveSession({ discoveredConfigOptions: configApi.asOptions(raw) });
    }).catch(() => {});
  };

  // Auto-open edit when panel is opened from sidebar with a pre-selected role name
  createEffect(() => {
    if (!props.initialRoleName) return;
    const role = userRoles().find((r) => r.roleName === props.initialRoleName);
    if (role && !selectedId()) openEdit(role);
  });

  const uniqueRoleName = (desired: string): string => {
    const existing = new Set(userRoles().map((r) => r.roleName.toLowerCase()));
    if (!existing.has(desired.toLowerCase())) return desired;
    const base = desired.replace(/_copy(\d+)?$/, "");
    let n = 2;
    let candidate = `${base}_copy`;
    while (existing.has(candidate.toLowerCase())) candidate = `${base}_copy${n++}`;
    return candidate;
  };

  const handleCreate = async () => {
    const name = uniqueRoleName(cName().trim());
    if (!name || saving()) return;
    if (/\s/.test(name)) {
      props.pushMessage("event", "Role name cannot contain spaces.");
      return;
    }
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      props.pushMessage("event", "Role name only allows letters, numbers, - and _.");
      return;
    }
    setSaving(true);
    const configMap: Record<string, string> = {};
    for (const [k, v] of Object.entries(cConfigSel())) { if (v) configMap[k] = v; }
    try {
      const saved = await roleApi.upsert({
        roleName: name, runtimeKind: cRuntime(),
        systemPrompt: cPrompt().trim() || "You are a helpful AI assistant.",
        model: cModel().trim() || null, mode: cMode().trim() || null,
        mcpServersJson: "[]", configOptionsJson: JSON.stringify(configMap),
        autoApprove: cAutoApprove(),
      } satisfies RoleUpsertInput);
      setCreating(false);
      setCConfigSel({}); setCConfigOpts([]);
      await props.refreshRoles();
      props.pushMessage("event", `role created: ${saved.roleName} (${saved.runtimeKind})`);
    } catch (e) { props.pushMessage("event", `Failed to create role: ${String(e)}`); }
    finally { setSaving(false); }
  };

  const handleSaveEdit = async () => {
    const role = editingRole();
    if (!role || saving()) return;
    let parsedMcp: unknown; let parsedCfg: unknown;
    try { parsedMcp = JSON.parse(eMcpJson().trim() || "[]"); if (!Array.isArray(parsedMcp)) throw new Error("must be array"); }
    catch (e) { props.pushMessage("event", `Invalid MCP JSON: ${String(e)}`); return; }
    try { parsedCfg = JSON.parse(eCfgJson().trim() || "{}"); if (!parsedCfg || typeof parsedCfg !== "object" || Array.isArray(parsedCfg)) throw new Error("must be object"); }
    catch (e) { props.pushMessage("event", `Invalid config JSON: ${String(e)}`); return; }
    setSaving(true);
    try {
      const saved = await roleApi.upsert({
        roleName: role.roleName, runtimeKind: role.runtimeKind,
        systemPrompt: ePrompt().trim(), model: eModel().trim() || null,
        mode: eMode().trim() || null, mcpServersJson: JSON.stringify(parsedMcp),
        configOptionsJson: JSON.stringify(parsedCfg), autoApprove: eAutoApprove(),
      } satisfies RoleUpsertInput);
      setSelectedId(null);
      await props.refreshRoles();
      props.pushMessage("event", `role saved: ${saved.roleName}`);
    } catch (e) { props.pushMessage("event", `Failed to save: ${String(e)}`); }
    finally { setSaving(false); }
  };

  const handleDelete = async (roleName: string) => {
    try {
      await roleApi.remove(roleName);
      if (editingRole()?.roleName === roleName) setSelectedId(null);
      setDeletingId(null);
      await props.refreshRoles();
    } catch (e) { props.pushMessage("event", `Failed to delete role: ${String(e)}`); }
  };

  const runtimeOptions = RUNTIMES.map((r) => ({ value: r, label: r }));

  // Config option helpers for edit form
  const editConfigOpts = createMemo(() => props.activeSession()?.discoveredConfigOptions ?? []);
  const editCfgMap = createMemo((): Record<string, string> => {
    try { return JSON.parse(eCfgJson() || "{}"); } catch { return {}; }
  });
  const updateEditCfg = (id: string, val: string) => {
    const map = { ...editCfgMap() };
    if (val) map[id] = val; else delete map[id];
    setECfgJson(JSON.stringify(map));
  };

  return (
    <div class="flex h-full">
      {/* List */}
      <div class="flex w-56 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <ActionButton
            label="+ New Role"
            variant="ghost"
            class="w-full"
            onClick={openCreate}
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={userRoles().length === 0}>
            <EmptyState icon="◎" title="No roles yet" sub="Create your first role" />
          </Show>
          <For each={userRoles()}>
            {(role) => {
              const color = () => RUNTIME_COLOR[role.runtimeKind] ?? "text-zinc-500";
              return (
                <div
                  onClick={() => openEdit(role)}
                  class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 cursor-default ${selectedId() === role.id ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"}`}
                >
                  <div class="flex items-center justify-between min-w-0 gap-1">
                    <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === role.id ? "text-zinc-100" : "text-zinc-300"}`}>{role.roleName}</span>
                    <Show when={deletingId() === role.roleName} fallback={
                      <button
                        onClick={(e) => { e.stopPropagation(); setDeletingId(role.roleName); }}
                        class="shrink-0 opacity-0 group-hover:opacity-100 text-zinc-700 hover:text-rose-400 transition-all"
                      >
                        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                      </button>
                    }>
                      <div class="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                        <button onClick={() => void handleDelete(role.roleName)} class="font-mono text-[9px] text-rose-400 hover:text-rose-300">del</button>
                        <button onClick={() => setDeletingId(null)} class="font-mono text-[9px] text-zinc-600 hover:text-zinc-400">✕</button>
                      </div>
                    </Show>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <span class={`font-mono text-[9px] ${color()}`}>{role.runtimeKind}</span>
                    <Show when={role.model}><span class="font-mono text-[9px] text-blue-400">{role.model}</span></Show>
                    <Show when={!role.autoApprove}><span class="font-mono text-[9px] text-amber-400">manual</span></Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </div>

      {/* Detail */}
      <div class="flex-1 overflow-y-auto p-5">
        {/* Create form */}
        <Show when={creating()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold text-zinc-300 uppercase tracking-widest">New Role</h3>
            <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
              <FieldRow label="Name">
                <div class="flex flex-col gap-1 w-full">
                  <TextInput
                    value={cName()}
                    onInput={setCName}
                    placeholder="e.g. Developer"
                    monospace
                    error={!!cName().trim() && !/^[A-Za-z0-9_-]+$/.test(cName().trim())}
                  />
                  <Show when={!!cName().trim() && !/^[A-Za-z0-9_-]+$/.test(cName().trim())}>
                    <span class="font-mono text-[9px] text-rose-400">only letters, numbers, - and _ (no spaces)</span>
                  </Show>
                </div>
              </FieldRow>
              <FieldRow label="Runtime">
                <InlineSelect value={cRuntime()} options={runtimeOptions} onChange={(v) => {
                  setCRuntime(v); setCModel(""); setCMode(""); setCConfigSel({});
                  void props.fetchConfigOptions(v).then(setCConfigOpts);
                }} />
              </FieldRow>
              <FieldRow label="Prompt">
                <TextInput value={cPrompt()} onInput={setCPrompt} placeholder="System prompt…" multiline rows={4} />
              </FieldRow>
              <FieldRow label="Model">
                <Show when={cConfigOpts().find((o) => o.category?.toLowerCase() === "model" || o.id.toLowerCase() === "model")} fallback={
                  <TextInput value={cModel()} onInput={setCModel} placeholder="Optional model override" monospace />
                }>
                  {(mo) => {
                    const values = () => flattenConfigValues(mo().options);
                    return (
                      <InlineSelect
                        value={cModel()}
                        options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                        onChange={setCModel}
                      />
                    );
                  }}
                </Show>
              </FieldRow>
              <Show when={cConfigOpts().find((o) => o.category?.toLowerCase() === "mode" || o.id.toLowerCase() === "mode")}>
                {(mo) => {
                  const values = () => flattenConfigValues(mo().options);
                  return (
                    <FieldRow label="Mode">
                      <InlineSelect
                        value={cMode()}
                        options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                        onChange={setCMode}
                      />
                    </FieldRow>
                  );
                }}
              </Show>
              <FieldRow label="Auto-approve">
                <label class="flex items-center gap-2 cursor-pointer">
                  <input type="checkbox" checked={cAutoApprove()} onChange={(e) => setCAutoApprove(e.currentTarget.checked)} class="rounded accent-emerald-500" />
                  <span class="font-mono text-[10px] text-zinc-500">auto-approve permissions</span>
                </label>
              </FieldRow>
              <Show when={cConfigOpts().length > 0}>
                <For each={cConfigOpts().filter((o) => {
                    const id = o.id.toLowerCase();
                    const cat = o.category?.toLowerCase();
                    return id !== "model" && id !== "mode" && cat !== "model" && cat !== "mode";
                  })}>
                  {(opt) => {
                    const values = () => flattenConfigValues(opt.options);
                    return (
                      <FieldRow label={opt.name}>
                        <InlineSelect
                          value={cConfigSel()[opt.id] ?? ""}
                          options={[{ value: "", label: `default: ${opt.currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.name }))]}
                          onChange={(val) => setCConfigSel((s) => ({ ...s, [opt.id]: val }))}
                        />
                      </FieldRow>
                    );
                  }}
                </For>
              </Show>
            </div>
            <div class="flex gap-2">
              <ActionButton label={saving() ? "Creating…" : "Create"} variant="primary" disabled={saving()} onClick={() => void handleCreate()} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>
          </div>
        </Show>

        {/* Edit form */}
        <Show when={!creating() && editingRole()}>
          {(role) => {
            const modelOpt = createMemo(() => editConfigOpts().find((o) => o.category?.toLowerCase() === "model" || o.id.toLowerCase() === "model"));
            const modeOpt = createMemo(() => editConfigOpts().find((o) => o.category?.toLowerCase() === "mode" || o.id.toLowerCase() === "mode"));
            const otherOpts = createMemo(() => editConfigOpts().filter((o) => {
              const id = o.id.toLowerCase();
              const cat = o.category?.toLowerCase();
              return id !== "model" && id !== "mode" && cat !== "model" && cat !== "mode";
            }));
            return (
              <div class="space-y-4">
                <div class="flex items-start justify-between gap-4">
                  <div>
                    <h2 class="font-mono text-sm font-bold text-zinc-100">{role().roleName}</h2>
                    <span class={`font-mono text-[10px] ${RUNTIME_COLOR[role().runtimeKind] ?? "text-zinc-500"}`}>{role().runtimeKind}</span>
                    <span class="font-mono text-[9px] text-zinc-700 ml-2">provider locked</span>
                  </div>
                  <Show when={deletingId() === role().roleName} fallback={
                    <ActionButton label="Delete" variant="danger" onClick={() => setDeletingId(role().roleName)} />
                  }>
                    <div class="flex items-center gap-2">
                      <ActionButton label="Confirm" variant="danger" onClick={() => void handleDelete(role().roleName)} />
                      <ActionButton label="Cancel" variant="ghost" onClick={() => setDeletingId(null)} />
                    </div>
                  </Show>
                </div>

                <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                  <FieldRow label="Prompt">
                    <TextInput value={ePrompt()} onInput={setEPrompt} placeholder="System prompt" multiline rows={4} />
                  </FieldRow>

                  <FieldRow label="Model">
                    <Show when={modelOpt()} fallback={
                      <TextInput value={eModel()} onInput={setEModel} placeholder="Optional model" monospace />
                    }>
                      {(mo) => {
                        const values = () => flattenConfigValues(mo().options);
                        return (
                          <InlineSelect
                            value={eModel()}
                            options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                            onChange={setEModel}
                          />
                        );
                      }}
                    </Show>
                  </FieldRow>

                  <Show when={modeOpt()}>
                    {(mo) => {
                      const values = () => flattenConfigValues(mo().options);
                      return (
                        <FieldRow label="Mode">
                          <InlineSelect
                            value={eMode()}
                            options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                            onChange={setEMode}
                          />
                        </FieldRow>
                      );
                    }}
                  </Show>

                  <For each={otherOpts()}>
                    {(opt) => {
                      const values = () => flattenConfigValues(opt.options);
                      return (
                        <FieldRow label={opt.name}>
                          <InlineSelect
                            value={editCfgMap()[opt.id] ?? ""}
                            options={[{ value: "", label: `default: ${opt.currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                            onChange={(val) => updateEditCfg(opt.id, val)}
                          />
                        </FieldRow>
                      );
                    }}
                  </For>

                  <FieldRow label="MCP">
                    <TextInput value={eMcpJson()} onInput={setEMcpJson} placeholder='[{"name":"..."}]' multiline rows={2} monospace />
                  </FieldRow>

                  <FieldRow label="Auto-approve">
                    <label class="flex items-center gap-2 cursor-pointer">
                      <input type="checkbox" checked={eAutoApprove()} onChange={(e) => setEAutoApprove(e.currentTarget.checked)} class="rounded accent-emerald-500" />
                      <span class="font-mono text-[10px] text-zinc-500">auto-approve permissions</span>
                    </label>
                  </FieldRow>
                </div>

                <div class="flex gap-2">
                  <ActionButton label={saving() ? "Saving…" : "Save"} variant="primary" disabled={saving()} onClick={() => void handleSaveEdit()} />
                  <ActionButton label="Cancel" variant="ghost" onClick={() => setSelectedId(null)} />
                </div>
              </div>
            );
          }}
        </Show>

        <Show when={!creating() && !editingRole()}>
          <EmptyState icon="◎" title="Select a role" sub="Or create a new one" />
        </Show>
      </div>
    </div>
  );
}
