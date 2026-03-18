import { invoke } from "@tauri-apps/api/core";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { Role, RoleUpsertInput, AppSession, AssistantRuntime, AcpConfigOption, AppSkill, AppMessage } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "./types";

type SelectOption = { value: string; label: string };

function Select(props: {
  value: string;
  options: SelectOption[];
  placeholder?: string;
  onChange: (v: string) => void;
  class?: string;
}) {
  const [open, setOpen] = createSignal(false);
  const selected = () => props.options.find((o) => o.value === props.value);

  const close = () => setOpen(false);
  const handleOutside = (e: MouseEvent) => {
    if (!(e.target as Element).closest("[data-select]")) close();
  };
  const toggle = () => {
    if (!open()) document.addEventListener("click", handleOutside, { once: true });
    setOpen((v) => !v);
  };

  onCleanup(() => document.removeEventListener("click", handleOutside));

  return (
    <div data-select class={`relative ${props.class ?? ""}`}>
      <button
        type="button"
        onClick={toggle}
        class={`flex h-8 w-full items-center justify-between gap-2 rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-left ${INTERACTIVE_MOTION} ${open() ? "border-zinc-600" : "hover:border-zinc-700"}`}
      >
        <span class={selected() ? "text-zinc-200" : "text-zinc-600"}>
          {selected()?.label ?? props.placeholder ?? "Select…"}
        </span>
        <svg class={`h-3.5 w-3.5 shrink-0 text-zinc-500 transition-transform ${open() ? "rotate-180" : ""}`} viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 4l4 4 4-4"/></svg>
      </button>
      <Show when={open()}>
        <div class="absolute left-0 right-0 top-full z-50 mt-0.5 max-h-52 overflow-y-auto rounded-lg border border-zinc-800 bg-zinc-950 shadow-xl shadow-black/40">
          <For each={props.options}>
            {(opt) => (
              <button
                type="button"
                onClick={() => { props.onChange(opt.value); close(); }}
                class={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs ${INTERACTIVE_MOTION} ${
                  opt.value === props.value
                    ? "bg-zinc-800/60 text-white"
                    : "text-zinc-300 hover:bg-zinc-800/60 hover:text-white"
                }`}
              >
                <span class={`h-1 w-1 shrink-0 rounded-full ${opt.value === props.value ? "bg-emerald-400" : "bg-transparent"}`} />
                {opt.label}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

type ConfigDrawerProps = {
  showDrawer: Accessor<boolean>;
  setShowDrawer: Setter<boolean>;
  assistants: Accessor<AssistantRuntime[]>;
  roles: Accessor<Role[]>;
  skills: Accessor<AppSkill[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  sendRaw: (text: string, silent?: boolean) => Promise<void>;
  refreshRoles: () => Promise<void>;
  refreshSkills: () => Promise<void>;
  pushMessage: (role: AppMessage["role"], text: string) => void;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
};

export default function ConfigDrawer(props: ConfigDrawerProps) {
  const [showRoleForm, setShowRoleForm] = createSignal(false);
  const [roleFormName, setRoleFormName] = createSignal("Developer");
  const [roleFormRuntime, setRoleFormRuntime] = createSignal("gemini-cli");
  const [roleFormPrompt, setRoleFormPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
  const [roleFormModel, setRoleFormModel] = createSignal("");
  const [roleFormMode, setRoleFormMode] = createSignal("");
  const [roleFormAutoApprove, setRoleFormAutoApprove] = createSignal(true);
  const [editingRoleId, setEditingRoleId] = createSignal<string | null>(null);
  const [editRolePrompt, setEditRolePrompt] = createSignal("");
  const [editRoleModel, setEditRoleModel] = createSignal("");
  const [editRoleMode, setEditRoleMode] = createSignal("");
  const [editRoleAutoApprove, setEditRoleAutoApprove] = createSignal(true);
  const [editRoleMcpServersJson, setEditRoleMcpServersJson] = createSignal("[]");
  const [editRoleConfigOptionsJson, setEditRoleConfigOptionsJson] = createSignal("{}");
  const [roleFormConfigSelections, setRoleFormConfigSelections] = createSignal<Record<string, string>>({});
  const [createFormConfigOptions, setCreateFormConfigOptions] = createSignal<AcpConfigOption[]>([]);

  const [showSkillForm, setShowSkillForm] = createSignal(false);
  const [skillFormName, setSkillFormName] = createSignal("");
  const [skillFormDescription, setSkillFormDescription] = createSignal("");
  const [skillFormContent, setSkillFormContent] = createSignal("");
  const [editingSkillId, setEditingSkillId] = createSignal<string | null>(null);

  const editingRole = (): Role | null => {
    const id = editingRoleId();
    if (!id) return null;
    return props.roles().find((role) => role.id === id) ?? null;
  };

  const closeRoleEditor = () => {
    setEditingRoleId(null);
  };

  const openRoleEditor = (role: Role) => {
    setShowRoleForm(false);
    setEditingRoleId(role.id);
    setEditRolePrompt(role.systemPrompt ?? "");
    setEditRoleModel(role.model ?? "");
    setEditRoleMode(role.mode ?? "");
    setEditRoleAutoApprove(role.autoApprove);
    setEditRoleMcpServersJson(role.mcpServersJson || "[]");
    setEditRoleConfigOptionsJson(role.configOptionsJson || "{}");
    props.patchActiveSession({ discoveredConfigOptions: [], configOptionsLoading: true });
    invoke<unknown[]>("prewarm_role_config_cmd", {
      runtimeKind: role.runtimeKind,
      roleName: role.roleName,
    }).then((raw) => {
      props.patchActiveSession({ discoveredConfigOptions: raw as AcpConfigOption[], configOptionsLoading: false });
    }).catch(() => props.patchActiveSession({ configOptionsLoading: false }));
  };

  const handleCreateRole = async () => {
    const name = roleFormName().trim();
    const runtime = roleFormRuntime() || props.activeSession()?.selectedAssistant || "gemini-cli";
    const prompt = roleFormPrompt().trim();
    if (!name) return;
    const model = roleFormModel().trim();
    const mode = roleFormMode();
    const autoApprove = roleFormAutoApprove();
    const cfgSelections = roleFormConfigSelections();
    // Build configOptionsJson from extra config selections
    const configMap: Record<string, string> = {};
    for (const [cfgId, cfgVal] of Object.entries(cfgSelections)) {
      if (cfgVal) configMap[cfgId] = cfgVal;
    }
    try {
      await invoke<Role>("upsert_role_cmd", {
        input: {
          roleName: name,
          runtimeKind: runtime,
          systemPrompt: prompt || "You are a helpful AI assistant.",
          model: model || null,
          mode: mode || null,
          mcpServersJson: "[]",
          configOptionsJson: JSON.stringify(configMap),
          autoApprove,
        } satisfies RoleUpsertInput,
      });
      setShowRoleForm(false);
      setRoleFormConfigSelections({});
      setCreateFormConfigOptions([]);
      await props.refreshRoles();
    } catch (e) {
      props.pushMessage("event", String(e));
    }
  };

  const handleSaveRoleEdit = async () => {
    const role = editingRole();
    if (!role) return;
    let parsedMcp: unknown;
    let parsedConfig: unknown;
    try {
      parsedMcp = JSON.parse(editRoleMcpServersJson().trim() || "[]");
      if (!Array.isArray(parsedMcp)) throw new Error("MCP servers JSON must be an array");
    } catch (e) {
      props.pushMessage("event", `Invalid MCP JSON: ${String(e)}`);
      return;
    }
    try {
      parsedConfig = JSON.parse(editRoleConfigOptionsJson().trim() || "{}");
      if (!parsedConfig || typeof parsedConfig !== "object" || Array.isArray(parsedConfig)) {
        throw new Error("Config options JSON must be an object");
      }
    } catch (e) {
      props.pushMessage("event", `Invalid config JSON: ${String(e)}`);
      return;
    }

    const payload: RoleUpsertInput = {
      roleName: role.roleName,
      runtimeKind: role.runtimeKind,
      systemPrompt: editRolePrompt().trim(),
      model: editRoleModel().trim() || null,
      mode: editRoleMode().trim() || null,
      mcpServersJson: JSON.stringify(parsedMcp),
      configOptionsJson: JSON.stringify(parsedConfig),
      autoApprove: editRoleAutoApprove(),
    };

    try {
      await invoke<Role>("upsert_role_cmd", { input: payload });
      closeRoleEditor();
      await props.refreshRoles();
      props.pushMessage("event", `role updated: ${role.roleName}`);
    } catch (e) {
      props.pushMessage("event", String(e));
    }
  };

  const saveSkill = async () => {
    const name = skillFormName().trim();
    if (!name) return;
    try {
      await invoke("upsert_app_skill", { input: { name, description: skillFormDescription().trim(), content: skillFormContent().trim() } });
      setShowSkillForm(false);
      setEditingSkillId(null);
      setSkillFormName("");
      setSkillFormDescription("");
      setSkillFormContent("");
      await props.refreshSkills();
    } catch (e) { props.pushMessage("event", String(e)); }
  };

  const deleteSkill = async (id: string) => {
    try {
      await invoke("delete_app_skill", { id });
      await props.refreshSkills();
    } catch (e) { props.pushMessage("event", String(e)); }
  };

  const openSkillEditor = (skill: AppSkill) => {
    setEditingSkillId(skill.id);
    setSkillFormName(skill.name);
    setSkillFormDescription(skill.description);
    setSkillFormContent(skill.content);
    setShowSkillForm(true);
  };

  return (
    <Show when={props.showDrawer()}>
      <div class="absolute inset-0 z-50 flex">
        <div class="flex-1" onClick={() => props.setShowDrawer(false)} />
        <div class="w-72 bg-[#0d0d10] border-l border-white/[0.06] flex flex-col overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3.5 border-b border-white/[0.05]">
            <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-500">Config</span>
            <button
              onClick={() => props.setShowDrawer(false)}
              class={`flex h-6 w-6 items-center justify-center rounded text-zinc-600 hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
            >
              <svg viewBox="0 0 12 12" class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 1l10 10M11 1L1 11"/></svg>
            </button>
          </div>
          <div class="flex-1 overflow-auto p-3 space-y-5">

            <AssistantSection
              assistants={props.assistants}
              activeSession={props.activeSession}
              patchActiveSession={props.patchActiveSession}
              pushMessage={props.pushMessage}
              fetchConfigOptions={props.fetchConfigOptions}
              refreshRoles={props.refreshRoles}
            />

            <RolesSection
              roles={props.roles}
              activeSession={props.activeSession}
              patchActiveSession={props.patchActiveSession}
              showRoleForm={showRoleForm}
              setShowRoleForm={setShowRoleForm}
              roleFormName={roleFormName}
              setRoleFormName={setRoleFormName}
              roleFormRuntime={roleFormRuntime}
              setRoleFormRuntime={setRoleFormRuntime}
              roleFormPrompt={roleFormPrompt}
              setRoleFormPrompt={setRoleFormPrompt}
              roleFormModel={roleFormModel}
              setRoleFormModel={setRoleFormModel}
              roleFormMode={roleFormMode}
              setRoleFormMode={setRoleFormMode}
              roleFormAutoApprove={roleFormAutoApprove}
              setRoleFormAutoApprove={setRoleFormAutoApprove}
              roleFormConfigSelections={roleFormConfigSelections}
              setRoleFormConfigSelections={setRoleFormConfigSelections}
              createFormConfigOptions={createFormConfigOptions}
              setCreateFormConfigOptions={setCreateFormConfigOptions}
              editingRoleId={editingRoleId}
              editingRole={editingRole}
              editRolePrompt={editRolePrompt}
              setEditRolePrompt={setEditRolePrompt}
              editRoleModel={editRoleModel}
              setEditRoleModel={setEditRoleModel}
              editRoleMode={editRoleMode}
              setEditRoleMode={setEditRoleMode}
              editRoleAutoApprove={editRoleAutoApprove}
              setEditRoleAutoApprove={setEditRoleAutoApprove}
              editRoleMcpServersJson={editRoleMcpServersJson}
              setEditRoleMcpServersJson={setEditRoleMcpServersJson}
              editRoleConfigOptionsJson={editRoleConfigOptionsJson}
              setEditRoleConfigOptionsJson={setEditRoleConfigOptionsJson}
              openRoleEditor={openRoleEditor}
              closeRoleEditor={closeRoleEditor}
              handleCreateRole={handleCreateRole}
              handleSaveRoleEdit={handleSaveRoleEdit}
              fetchConfigOptions={props.fetchConfigOptions}
            />

            <SkillsSection
              skills={props.skills}
              showSkillForm={showSkillForm}
              setShowSkillForm={setShowSkillForm}
              skillFormName={skillFormName}
              setSkillFormName={setSkillFormName}
              skillFormDescription={skillFormDescription}
              setSkillFormDescription={setSkillFormDescription}
              skillFormContent={skillFormContent}
              setSkillFormContent={setSkillFormContent}
              editingSkillId={editingSkillId}
              setEditingSkillId={setEditingSkillId}
              saveSkill={saveSkill}
              deleteSkill={deleteSkill}
              openSkillEditor={openSkillEditor}
            />

            <div class="flex flex-wrap gap-0.5 border-t border-white/[0.05] pt-3">
              <button onClick={() => void props.sendRaw("/workflow list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-600 hover:text-zinc-300 hover:bg-white/[0.04] ${INTERACTIVE_MOTION}`}>workflows</button>
              <button onClick={() => void props.sendRaw("/context list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-600 hover:text-zinc-300 hover:bg-white/[0.04] ${INTERACTIVE_MOTION}`}>context</button>
              <button onClick={() => void props.sendRaw("/session list")} class={`min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-600 hover:text-zinc-300 hover:bg-white/[0.04] ${INTERACTIVE_MOTION}`}>sessions</button>
            </div>

          </div>
        </div>
      </div>
    </Show>
  );
}

const UNION_ASSISTANT_ROLE = "UnionAIAssistant";

type AssistantSectionProps = {
  assistants: Accessor<AssistantRuntime[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  pushMessage: (role: AppMessage["role"], text: string) => void;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  refreshRoles: () => Promise<void>;
};

function AssistantSection(props: AssistantSectionProps) {
  const [showConfig, setShowConfig] = createSignal(false);

  return (
    <div>
      <div class="mb-2 flex items-center justify-between">
        <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-600">Assistant</span>
        <button
          onClick={() => setShowConfig((v) => !v)}
          class={`min-h-6 rounded border border-zinc-800 px-2 py-0.5 text-[10px] text-zinc-600 hover:border-zinc-700 hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
        >
          {showConfig() ? "close" : "config"}
        </button>
      </div>
      <div class="space-y-0.5">
        <For each={props.assistants()}>
          {(a) => (
            <button
              class={`flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-xs ${INTERACTIVE_MOTION} ${
                props.activeSession()?.selectedAssistant === a.key
                  ? "bg-zinc-800/80 text-white"
                  : "hover:bg-zinc-900/60 text-zinc-400"
              } ${!a.available ? "opacity-40" : ""}`}
              onClick={() => a.available && props.patchActiveSession({ selectedAssistant: a.key })}
            >
              <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-500"}`} />
              <span class="flex-1 font-medium">{a.label}</span>
              <Show when={a.version}><span class="text-[10px] text-zinc-600">v{a.version}</span></Show>
            </button>
          )}
        </For>
      </div>
      <Show when={showConfig()}>
        <AssistantConfigPanel
          activeSession={props.activeSession}
          pushMessage={props.pushMessage}
          fetchConfigOptions={props.fetchConfigOptions}
          refreshRoles={props.refreshRoles}
          onClose={() => setShowConfig(false)}
        />
      </Show>
    </div>
  );
}

type AssistantConfigPanelProps = {
  activeSession: Accessor<AppSession | null>;
  pushMessage: (role: AppMessage["role"], text: string) => void;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  refreshRoles: () => Promise<void>;
  onClose: () => void;
};

function AssistantConfigPanel(props: AssistantConfigPanelProps) {
  const [prompt, setPrompt] = createSignal("");
  const [model, setModel] = createSignal("");
  const [mode, setMode] = createSignal("");
  const [configOptionsJson, setConfigOptionsJson] = createSignal("{}");
  const [configOptions, setConfigOptions] = createSignal<AcpConfigOption[]>([]);
  const [loading, setLoading] = createSignal(true);

  const runtimeKind = () => props.activeSession()?.selectedAssistant ?? "claude-code";

  const currentCfg = createMemo((): Record<string, string> => {
    try { return JSON.parse(configOptionsJson() || "{}"); } catch { return {}; }
  });
  const updateCfg = (id: string, val: string) => {
    const map = { ...currentCfg() };
    if (val) map[id] = val; else delete map[id];
    setConfigOptionsJson(JSON.stringify(map));
  };
  const modelOpt = createMemo(() => configOptions().find((o) => o.category === "model" || o.id === "model"));
  const modeOpt = createMemo(() => configOptions().find((o) => o.category === "mode" || o.id === "mode"));
  const otherOpts = createMemo(() => configOptions().filter((o) => o.id !== "model" && o.id !== "mode" && o.category !== "model" && o.category !== "mode"));

  // Load existing UnionAIAssistant role on mount
  const init = async () => {
    setLoading(true);
    try {
      const roles = await invoke<Role[]>("list_roles");
      const existing = roles.find((r) => r.roleName === UNION_ASSISTANT_ROLE);
      if (existing) {
        setPrompt(existing.systemPrompt ?? "");
        setModel(existing.model ?? "");
        setMode(existing.mode ?? "");
        setConfigOptionsJson(existing.configOptionsJson || "{}");
      }
      const opts = await props.fetchConfigOptions(runtimeKind(), UNION_ASSISTANT_ROLE);
      setConfigOptions(opts);
    } catch { /* ignore */ }
    setLoading(false);
  };
  createEffect(() => {
    void runtimeKind();
    void init();
  });

  const handleSave = async () => {
    try {
      await invoke<Role>("upsert_role_cmd", {
        input: {
          roleName: UNION_ASSISTANT_ROLE,
          runtimeKind: runtimeKind(),
          systemPrompt: prompt().trim(),
          model: model().trim() || null,
          mode: mode() || null,
          mcpServersJson: "[]",
          configOptionsJson: configOptionsJson(),
          autoApprove: true,
        } satisfies RoleUpsertInput,
      });
      await props.refreshRoles();
      props.pushMessage("event", "assistant config saved");
      props.onClose();
    } catch (e) {
      props.pushMessage("event", String(e));
    }
  };

  return (
    <div class="mt-2 space-y-2 rounded-xl border border-zinc-800/60 bg-zinc-950/60 p-3">
      <Show when={loading()}>
        <div class="text-[10px] text-zinc-600 italic">Loading...</div>
      </Show>
      <Show when={!loading()}>
        <div class="text-[10px] text-zinc-600">runtime: {runtimeKind()}</div>
        <textarea
          value={prompt()}
          onInput={(e) => setPrompt(e.currentTarget.value)}
          rows={3}
          placeholder="System prompt (optional)"
          class="w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 py-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
        />
        <Show when={modelOpt()} fallback={
          <input value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none" />
        }>
          {(mo) => {
            const values = () => flattenConfigValues(mo().options);
            return (
              <Select
                value={model()}
                options={[{ value: "", label: `Model (default: ${mo().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                onChange={setModel}
              />
            );
          }}
        </Show>
        <Show when={modeOpt()}>
          {(mo) => {
            const values = () => flattenConfigValues(mo().options);
            return (
              <Select
                value={mode()}
                options={[{ value: "", label: `Mode (default: ${mo().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                onChange={setMode}
              />
            );
          }}
        </Show>
        <For each={otherOpts()}>
          {(opt) => {
            const values = () => flattenConfigValues(opt.options);
            const sel = () => currentCfg()[opt.id] ?? "";
            return (
              <div class="flex flex-col gap-0.5">
                <label class="text-[10px] text-zinc-600 mb-0.5">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                <Select
                  value={sel()}
                  options={[{ value: "", label: `(default: ${opt.currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                  onChange={(val) => updateCfg(opt.id, val)}
                />
              </div>
            );
          }}
        </For>
        <button
          onClick={() => void handleSave()}
          class={`h-8 w-full rounded-lg bg-white text-xs font-medium text-zinc-950 ${INTERACTIVE_MOTION}`}
        >
          Save
        </button>
      </Show>
    </div>
  );
}

type RolesSectionProps = {
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  showRoleForm: Accessor<boolean>;
  setShowRoleForm: Setter<boolean>;
  roleFormName: Accessor<string>;
  setRoleFormName: Setter<string>;
  roleFormRuntime: Accessor<string>;
  setRoleFormRuntime: Setter<string>;
  roleFormPrompt: Accessor<string>;
  setRoleFormPrompt: Setter<string>;
  roleFormModel: Accessor<string>;
  setRoleFormModel: Setter<string>;
  roleFormMode: Accessor<string>;
  setRoleFormMode: Setter<string>;
  roleFormAutoApprove: Accessor<boolean>;
  setRoleFormAutoApprove: Setter<boolean>;
  roleFormConfigSelections: Accessor<Record<string, string>>;
  setRoleFormConfigSelections: Setter<Record<string, string>>;
  createFormConfigOptions: Accessor<AcpConfigOption[]>;
  setCreateFormConfigOptions: Setter<AcpConfigOption[]>;
  editingRoleId: Accessor<string | null>;
  editingRole: () => Role | null;
  editRolePrompt: Accessor<string>;
  setEditRolePrompt: Setter<string>;
  editRoleModel: Accessor<string>;
  setEditRoleModel: Setter<string>;
  editRoleMode: Accessor<string>;
  setEditRoleMode: Setter<string>;
  editRoleAutoApprove: Accessor<boolean>;
  setEditRoleAutoApprove: Setter<boolean>;
  editRoleMcpServersJson: Accessor<string>;
  setEditRoleMcpServersJson: Setter<string>;
  editRoleConfigOptionsJson: Accessor<string>;
  setEditRoleConfigOptionsJson: Setter<string>;
  openRoleEditor: (role: Role) => void;
  closeRoleEditor: () => void;
  handleCreateRole: () => Promise<void>;
  handleSaveRoleEdit: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
};

function RolesSection(props: RolesSectionProps) {
  const userRoles = createMemo(() => props.roles().filter((r) => r.roleName !== UNION_ASSISTANT_ROLE));

  return (
    <div>
      <div class="mb-2 flex items-center justify-between">
        <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-600">Roles</span>
        <button
          onClick={() => props.setShowRoleForm((v) => {
            const next = !v;
            if (next) props.closeRoleEditor();
            if (next && props.activeSession()?.selectedAssistant) props.setRoleFormRuntime(props.activeSession()!.selectedAssistant!);
            return next;
          })}
          class={`min-h-6 rounded border border-zinc-800 px-2 py-0.5 text-[10px] text-zinc-600 hover:border-zinc-700 hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
        >
          {props.showRoleForm() ? "cancel" : "+ role"}
        </button>
      </div>

      <Show when={props.showRoleForm()}>
        <CreateRoleForm
          roleFormName={props.roleFormName}
          setRoleFormName={props.setRoleFormName}
          roleFormRuntime={props.roleFormRuntime}
          setRoleFormRuntime={props.setRoleFormRuntime}
          roleFormPrompt={props.roleFormPrompt}
          setRoleFormPrompt={props.setRoleFormPrompt}
          roleFormModel={props.roleFormModel}
          setRoleFormModel={props.setRoleFormModel}
          roleFormMode={props.roleFormMode}
          setRoleFormMode={props.setRoleFormMode}
          roleFormAutoApprove={props.roleFormAutoApprove}
          setRoleFormAutoApprove={props.setRoleFormAutoApprove}
          roleFormConfigSelections={props.roleFormConfigSelections}
          setRoleFormConfigSelections={props.setRoleFormConfigSelections}
          createFormConfigOptions={props.createFormConfigOptions}
          setCreateFormConfigOptions={props.setCreateFormConfigOptions}
          handleCreateRole={props.handleCreateRole}
          fetchConfigOptions={props.fetchConfigOptions}
        />
      </Show>

      <Show when={props.editingRole()}>
        {(role) => (
          <EditRoleForm
            role={role}
            activeSession={props.activeSession}
            editRolePrompt={props.editRolePrompt}
            setEditRolePrompt={props.setEditRolePrompt}
            editRoleModel={props.editRoleModel}
            setEditRoleModel={props.setEditRoleModel}
            editRoleMode={props.editRoleMode}
            setEditRoleMode={props.setEditRoleMode}
            editRoleAutoApprove={props.editRoleAutoApprove}
            setEditRoleAutoApprove={props.setEditRoleAutoApprove}
            editRoleMcpServersJson={props.editRoleMcpServersJson}
            setEditRoleMcpServersJson={props.setEditRoleMcpServersJson}
            editRoleConfigOptionsJson={props.editRoleConfigOptionsJson}
            setEditRoleConfigOptionsJson={props.setEditRoleConfigOptionsJson}
            closeRoleEditor={props.closeRoleEditor}
            handleSaveRoleEdit={props.handleSaveRoleEdit}
          />
        )}
      </Show>

      <div class="space-y-0.5">
        <For each={userRoles()}>
          {(role) => {
            const cfgKeyCount = createMemo(() => {
              try {
                const c = JSON.parse(role.configOptionsJson || "{}");
                return Object.keys(c).filter((k) => k !== "model" && k !== "mode" && c[k]).length;
              } catch { return 0; }
            });
            const runtimeColor = () => RUNTIME_COLOR[role.runtimeKind] ?? "text-zinc-500";
            return (
              <div class="group relative flex items-start gap-2 rounded-lg px-2.5 py-2 motion-safe:transition-all motion-safe:duration-150 hover:bg-zinc-900/60">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-1.5 flex-wrap">
                    <span class="text-xs font-semibold text-zinc-200">{role.roleName}</span>
                    <span class={`rounded-full px-1.5 py-px text-[9px] font-medium bg-zinc-800 ${runtimeColor()}`}>{role.runtimeKind}</span>
                    <Show when={role.model}><span class="rounded-full bg-blue-500/15 px-1.5 text-[9px] font-medium text-blue-300">{role.model}</span></Show>
                    <Show when={role.mode}><span class="rounded-full bg-indigo-500/15 px-1.5 text-[9px] font-medium text-indigo-300">{role.mode}</span></Show>
                    <Show when={!role.autoApprove}><span class="rounded-full bg-amber-500/15 px-1.5 text-[9px] font-medium text-amber-300">manual</span></Show>
                    <Show when={cfgKeyCount() > 0}><span class="rounded-full bg-teal-500/15 px-1.5 text-[9px] font-medium text-teal-300">{cfgKeyCount()} cfg</span></Show>
                  </div>
                  <p class="mt-0.5 truncate text-[10px] text-zinc-600">{role.systemPrompt}</p>
                </div>
                <div class="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 motion-safe:transition-opacity motion-safe:duration-150">
                  <button
                    onClick={() => props.openRoleEditor(role)}
                    class={`rounded px-1.5 py-0.5 text-[10px] ${props.editingRoleId() === role.id ? "text-blue-400" : "text-zinc-500 hover:text-zinc-200"} ${INTERACTIVE_MOTION}`}
                  >
                    {props.editingRoleId() === role.id ? "editing" : "edit"}
                  </button>
                </div>
              </div>
            );
          }}
        </For>
        <Show when={userRoles().length === 0 && !props.showRoleForm()}>
          <p class="rounded-lg border border-dashed border-zinc-800 p-3 text-center text-[10px] text-zinc-600">
            No roles yet. Click "+ role" to add one.
          </p>
        </Show>
      </div>
    </div>
  );
}

type CreateRoleFormProps = {
  roleFormName: Accessor<string>;
  setRoleFormName: Setter<string>;
  roleFormRuntime: Accessor<string>;
  setRoleFormRuntime: Setter<string>;
  roleFormPrompt: Accessor<string>;
  setRoleFormPrompt: Setter<string>;
  roleFormModel: Accessor<string>;
  setRoleFormModel: Setter<string>;
  roleFormMode: Accessor<string>;
  setRoleFormMode: Setter<string>;
  roleFormAutoApprove: Accessor<boolean>;
  setRoleFormAutoApprove: Setter<boolean>;
  roleFormConfigSelections: Accessor<Record<string, string>>;
  setRoleFormConfigSelections: Setter<Record<string, string>>;
  createFormConfigOptions: Accessor<AcpConfigOption[]>;
  setCreateFormConfigOptions: Setter<AcpConfigOption[]>;
  handleCreateRole: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
};

function CreateRoleForm(props: CreateRoleFormProps) {
  return (
    <div class="mb-2 space-y-2 rounded-xl border border-zinc-800/60 bg-zinc-950/60 p-3">
      <input
        value={props.roleFormName()}
        onInput={(e) => props.setRoleFormName(e.currentTarget.value)}
        placeholder="Role name (e.g. Developer)"
        class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
      />
      <Select
        value={props.roleFormRuntime()}
        options={RUNTIMES.map((r) => ({ value: r, label: r }))}
        onChange={(val) => {
          props.setRoleFormRuntime(val);
          props.setRoleFormConfigSelections({});
          void props.fetchConfigOptions(val).then(props.setCreateFormConfigOptions);
        }}
      />
      <textarea
        value={props.roleFormPrompt()}
        onInput={(e) => props.setRoleFormPrompt(e.currentTarget.value)}
        rows={3}
        placeholder="System prompt for this role..."
        class="w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 py-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
      />
      <For each={props.createFormConfigOptions()}>
        {(opt) => {
          const values = () => flattenConfigValues(opt.options);
          const sel = () => props.roleFormConfigSelections()[opt.id] ?? "";
          const isModel = () => opt.category === "model" || opt.id === "model";
          const isMode = () => opt.category === "mode" || opt.id === "mode";
          return (
            <Show when={!isModel() && !isMode()}>
              <div class="flex flex-col gap-0.5">
                <label class="text-[10px] text-zinc-600 mb-0.5">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                <Select
                  value={sel()}
                  options={[{ value: "", label: `(default: ${opt.currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                  onChange={(val) => props.setRoleFormConfigSelections((prev) => ({ ...prev, [opt.id]: val }))}
                />
              </div>
            </Show>
          );
        }}
      </For>
      <Show when={props.createFormConfigOptions().some((o) => o.category === "model" || o.id === "model")} fallback={
        <input value={props.roleFormModel()} onInput={(e) => props.setRoleFormModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none" />
      }>
        {(() => {
          const modelOpt = () => props.createFormConfigOptions().find((o) => o.category === "model" || o.id === "model")!;
          const values = () => flattenConfigValues(modelOpt().options);
          return (
            <Select
              value={props.roleFormModel()}
              options={[{ value: "", label: `Model (default: ${modelOpt().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
              onChange={props.setRoleFormModel}
            />
          );
        })()}
      </Show>
      <Show when={props.createFormConfigOptions().some((o) => o.category === "mode" || o.id === "mode")} fallback={
        <Select
          value={props.roleFormMode()}
          options={[{ value: "", label: "Mode (default)" }, { value: "plan", label: "plan" }, { value: "act", label: "act" }, { value: "auto", label: "auto" }]}
          onChange={props.setRoleFormMode}
        />
      }>
        {(() => {
          const modeOpt = () => props.createFormConfigOptions().find((o) => o.category === "mode" || o.id === "mode")!;
          const values = () => flattenConfigValues(modeOpt().options);
          return (
            <Select
              value={props.roleFormMode()}
              options={[{ value: "", label: `Mode (default: ${modeOpt().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
              onChange={props.setRoleFormMode}
            />
          );
        })()}
      </Show>
      <label class="flex items-center gap-2 text-[10px] text-zinc-600 cursor-pointer select-none">
        <input
          type="checkbox"
          checked={props.roleFormAutoApprove()}
          onChange={(e) => props.setRoleFormAutoApprove(e.currentTarget.checked)}
          class="rounded accent-emerald-500"
        />
        Auto-approve permissions
      </label>
      <button
        onClick={() => void props.handleCreateRole()}
        class={`h-8 w-full rounded-lg bg-white text-xs font-medium text-zinc-950 ${INTERACTIVE_MOTION}`}
      >
        Create Role
      </button>
    </div>
  );
}

type EditRoleFormProps = {
  role: Accessor<Role>;
  activeSession: Accessor<AppSession | null>;
  editRolePrompt: Accessor<string>;
  setEditRolePrompt: Setter<string>;
  editRoleModel: Accessor<string>;
  setEditRoleModel: Setter<string>;
  editRoleMode: Accessor<string>;
  setEditRoleMode: Setter<string>;
  editRoleAutoApprove: Accessor<boolean>;
  setEditRoleAutoApprove: Setter<boolean>;
  editRoleMcpServersJson: Accessor<string>;
  setEditRoleMcpServersJson: Setter<string>;
  editRoleConfigOptionsJson: Accessor<string>;
  setEditRoleConfigOptionsJson: Setter<string>;
  closeRoleEditor: () => void;
  handleSaveRoleEdit: () => Promise<void>;
};

function EditRoleForm(props: EditRoleFormProps) {
  const opts = createMemo(() => props.activeSession()?.discoveredConfigOptions ?? []);
  const currentCfg = createMemo((): Record<string, string> => {
    try { return JSON.parse(props.editRoleConfigOptionsJson() || "{}"); } catch { return {}; }
  });
  const updateCfg = (id: string, val: string) => {
    const map = { ...currentCfg() };
    if (val) map[id] = val; else delete map[id];
    props.setEditRoleConfigOptionsJson(JSON.stringify(map));
  };
  const modelOpt = createMemo(() => opts().find((o) => o.category === "model" || o.id === "model"));
  const modeOpt = createMemo(() => opts().find((o) => o.category === "mode" || o.id === "mode"));
  const otherOpts = createMemo(() => opts().filter((o) => o.id !== "model" && o.id !== "mode" && o.category !== "model" && o.category !== "mode"));

  return (
    <div class="mb-2 space-y-2 rounded-xl border border-zinc-800/60 bg-zinc-950/60 p-3">
      <div class="flex items-center justify-between">
        <div class="text-xs font-semibold text-zinc-200">Edit {props.role().roleName}</div>
        <button
          onClick={props.closeRoleEditor}
          class={`rounded px-1.5 py-0.5 text-[10px] text-zinc-500 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
        >
          close
        </button>
      </div>
      <div class="text-[10px] text-zinc-600">provider locked: {props.role().runtimeKind}</div>
      <textarea
        value={props.editRolePrompt()}
        onInput={(e) => props.setEditRolePrompt(e.currentTarget.value)}
        rows={3}
        placeholder="System prompt"
        class="w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 py-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
      />
      <Show when={modelOpt()} fallback={
        <input value={props.editRoleModel()} onInput={(e) => props.setEditRoleModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none" />
      }>
        {(mo) => {
          const values = () => flattenConfigValues(mo().options);
          return (
            <Select
              value={props.editRoleModel()}
              options={[{ value: "", label: `Model (default: ${mo().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]  }
              onChange={(e) => props.setEditRoleModel(e)}
            />
          );
        }}
      </Show>
      <Show when={modeOpt()}>
        {(mo) => {
          const values = () => flattenConfigValues(mo().options);
          return (
            <Select
              value={props.editRoleMode()}
              options={[{ value: "", label: `Mode (default: ${mo().currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
              onChange={(e) => props.setEditRoleMode(e)}
            />
          );
        }}
      </Show>
      <For each={otherOpts()}>
        {(opt) => {
          const values = () => flattenConfigValues(opt.options);
          const sel = () => currentCfg()[opt.id] ?? "";
          return (
            <div class="flex flex-col gap-0.5">
              <label class="text-[10px] text-zinc-600 mb-0.5">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
              <Select
                value={sel()}
                options={[{ value: "", label: `(default: ${opt.currentValue})` }, ...values().map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))]}
                onChange={(val) => updateCfg(opt.id, val)}
              />
            </div>
          );
        }}
      </For>
      <Show when={opts().length === 0}>
        <div class="text-[10px] text-zinc-600 italic">
          {props.activeSession()?.configOptionsLoading ? "Loading config options from agent..." : "No config options available for this agent."}
        </div>
      </Show>
      <details class="group">
        <summary class="text-[10px] text-zinc-600 hover:text-zinc-400 flex items-center gap-1.5 cursor-pointer select-none list-none">
          <svg class="h-2.5 w-2.5 shrink-0 transition-transform group-open:rotate-90" viewBox="0 0 8 8" fill="currentColor"><path d="M2 1l4 3-4 3V1z"/></svg>
          MCP servers
        </summary>
        <textarea
          value={props.editRoleMcpServersJson()}
          onInput={(e) => props.setEditRoleMcpServersJson(e.currentTarget.value)}
          rows={2}
          placeholder='[{"name":"..." }]'
          class="mt-1.5 w-full resize-y rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 py-2 text-xs font-mono text-zinc-300 placeholder:text-zinc-700 focus:border-zinc-600 focus:outline-none"
        />
      </details>
      <label class="flex items-center gap-2 text-[10px] text-zinc-600 cursor-pointer select-none">
        <input
          type="checkbox"
          checked={props.editRoleAutoApprove()}
          onChange={(e) => props.setEditRoleAutoApprove(e.currentTarget.checked)}
          class="rounded accent-emerald-500"
        />
        Auto-approve permissions
      </label>
      <button
        onClick={() => void props.handleSaveRoleEdit()}
        class={`h-8 w-full rounded-lg bg-white text-xs font-medium text-zinc-950 ${INTERACTIVE_MOTION}`}
      >
        Save Role
      </button>
    </div>
  );
}

type SkillsSectionProps = {
  skills: Accessor<AppSkill[]>;
  showSkillForm: Accessor<boolean>;
  setShowSkillForm: Setter<boolean>;
  skillFormName: Accessor<string>;
  setSkillFormName: Setter<string>;
  skillFormDescription: Accessor<string>;
  setSkillFormDescription: Setter<string>;
  skillFormContent: Accessor<string>;
  setSkillFormContent: Setter<string>;
  editingSkillId: Accessor<string | null>;
  setEditingSkillId: Setter<string | null>;
  saveSkill: () => Promise<void>;
  deleteSkill: (id: string) => Promise<void>;
  openSkillEditor: (skill: AppSkill) => void;
};

function SkillsSection(props: SkillsSectionProps) {
  return (
    <div>
      <div class="mb-2 flex items-center justify-between">
        <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-600">Skills</span>
        <button
          onClick={() => {
            props.setEditingSkillId(null);
            props.setSkillFormName("");
            props.setSkillFormDescription("");
            props.setSkillFormContent("");
            props.setShowSkillForm((v) => !v);
          }}
          class={`min-h-6 rounded border border-zinc-800 px-2 py-0.5 text-[10px] text-zinc-600 hover:border-zinc-700 hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
        >
          {props.showSkillForm() && !props.editingSkillId() ? "cancel" : "+ skill"}
        </button>
      </div>

      <Show when={props.showSkillForm()}>
        <div class="mb-2 space-y-2 rounded-xl border border-zinc-800/60 bg-zinc-950/60 p-3">
          <input
            value={props.skillFormName()}
            onInput={(e) => props.setSkillFormName(e.currentTarget.value)}
            placeholder="Skill name (e.g. code-review)"
            class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
          />
          <input
            value={props.skillFormDescription()}
            onInput={(e) => props.setSkillFormDescription(e.currentTarget.value)}
            placeholder="Short description"
            class="h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
          />
          <textarea
            value={props.skillFormContent()}
            onInput={(e) => props.setSkillFormContent(e.currentTarget.value)}
            rows={4}
            placeholder="Skill content - prompt instructions, templates, context..."
            class="w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 px-2.5 py-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:border-zinc-600 focus:outline-none"
          />
          <button
            onClick={props.saveSkill}
            class={`w-full rounded-lg bg-white py-2 text-xs font-medium text-zinc-950 ${INTERACTIVE_MOTION}`}
          >
            {props.editingSkillId() ? "Save changes" : "Create skill"}
          </button>
        </div>
      </Show>

      <div class="space-y-0.5">
        <For each={props.skills()}>
          {(skill) => (
            <div class="group rounded-lg px-2.5 py-2 hover:bg-zinc-900/60 motion-safe:transition-colors motion-safe:duration-150">
              <div class="flex items-center justify-between gap-1">
                <span class="text-xs font-medium text-zinc-300">#{skill.name}</span>
                <div class="flex gap-1 opacity-0 group-hover:opacity-100 motion-safe:transition-opacity motion-safe:duration-150">
                  <button
                    onClick={() => props.openSkillEditor(skill)}
                    class={`rounded px-1.5 py-0.5 text-[10px] text-zinc-500 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
                  >edit</button>
                  <button
                    onClick={() => void props.deleteSkill(skill.id)}
                    class={`rounded px-1.5 py-0.5 text-[10px] text-rose-500 hover:text-rose-400 ${INTERACTIVE_MOTION}`}
                  >del</button>
                </div>
              </div>
              <Show when={skill.description}>
                <p class="mt-0.5 text-[10px] text-zinc-600 truncate">{skill.description}</p>
              </Show>
            </div>
          )}
        </For>
        <Show when={props.skills().length === 0 && !props.showSkillForm()}>
          <p class="rounded-lg border border-dashed border-zinc-800 p-3 text-center text-[10px] text-zinc-600">
            No skills yet. Click "+ skill" to add one.
          </p>
        </Show>
      </div>
    </div>
  );
}
