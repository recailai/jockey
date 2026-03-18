import { invoke } from "@tauri-apps/api/core";
import { For, Show, createMemo, createSignal } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { Role, RoleUpsertInput, AppSession, AssistantRuntime, AcpConfigOption, AppSkill, AppMessage } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "./types";

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
  fetchConfigOptions: (runtimeKey: string, roleName?: string, teamId?: string | null) => Promise<AcpConfigOption[]>;
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
      teamId: role.teamId,
    }).then((raw) => {
      props.patchActiveSession({ discoveredConfigOptions: raw as AcpConfigOption[], configOptionsLoading: false });
    }).catch(() => props.patchActiveSession({ configOptionsLoading: false }));
  };

  const handleCreateRole = async () => {
    const name = roleFormName().trim();
    const runtime = roleFormRuntime() || props.activeSession()?.selectedAssistant || "gemini-cli";
    const prompt = roleFormPrompt().trim();
    if (!name) return;
    setShowRoleForm(false);
    await props.sendRaw(`/app_role bind ${name} ${runtime} ${prompt || "You are a helpful AI assistant."}`);
    const model = roleFormModel().trim();
    const mode = roleFormMode();
    const autoApprove = roleFormAutoApprove();
    if (model) await props.sendRaw(`/app_role edit ${name} model ${model}`, true);
    if (mode) await props.sendRaw(`/app_role edit ${name} mode ${mode}`, true);
    if (!autoApprove) await props.sendRaw(`/app_role edit ${name} auto-approve false`, true);
    const cfgSelections = roleFormConfigSelections();
    for (const [cfgId, cfgVal] of Object.entries(cfgSelections)) {
      if (cfgVal) await props.sendRaw(`/app_role edit ${name} config ${cfgId} ${cfgVal}`, true);
    }
    setRoleFormConfigSelections({});
    setCreateFormConfigOptions([]);
    await props.refreshRoles();
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
      teamId: role.teamId,
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
        <div class="w-72 bg-[#111114] border-l border-white/[0.07] flex flex-col overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3 border-b border-white/[0.06]">
            <span class="text-xs font-semibold text-zinc-300">Config</span>
            <button onClick={() => props.setShowDrawer(false)} class="text-zinc-500 hover:text-zinc-300 text-lg leading-none">x</button>
          </div>
          <div class="flex-1 overflow-auto p-3 space-y-4">

            <AssistantSection
              assistants={props.assistants}
              activeSession={props.activeSession}
              patchActiveSession={props.patchActiveSession}
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
              <button onClick={() => void props.sendRaw("/workflow list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">workflows</button>
              <button onClick={() => void props.sendRaw("/context list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">context</button>
              <button onClick={() => void props.sendRaw("/session list")} class="min-h-8 rounded-md px-2.5 py-1 text-xs text-zinc-500 hover:text-white hover:bg-white/[0.05] motion-safe:transition-colors motion-safe:duration-150">sessions</button>
            </div>

          </div>
        </div>
      </div>
    </Show>
  );
}

type AssistantSectionProps = {
  assistants: Accessor<AssistantRuntime[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
};

function AssistantSection(props: AssistantSectionProps) {
  return (
    <div>
      <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Assistant</div>
      <div class="space-y-1">
        <For each={props.assistants()}>
          {(a) => (
            <button
              class={`flex min-h-9 w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm ${INTERACTIVE_MOTION} ${
                props.activeSession()?.selectedAssistant === a.key
                  ? "bg-white/[0.08] text-white"
                  : "hover:bg-white/[0.05] text-zinc-400"
              } ${!a.available ? "opacity-40" : ""}`}
              onClick={() => a.available && props.patchActiveSession({ selectedAssistant: a.key })}
            >
              <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
              <span class="flex-1 font-medium">{a.label}</span>
              <Show when={a.version}><span class="text-xs text-zinc-500">v{a.version}</span></Show>
            </button>
          )}
        </For>
      </div>
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
  fetchConfigOptions: (runtimeKey: string, roleName?: string, teamId?: string | null) => Promise<AcpConfigOption[]>;
};

function RolesSection(props: RolesSectionProps) {
  return (
    <div>
      <div class="mb-2 flex items-center justify-between">
        <span class="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Roles</span>
        <button
          onClick={() => props.setShowRoleForm((v) => {
            const next = !v;
            if (next) props.closeRoleEditor();
            if (next && props.activeSession()?.selectedAssistant) props.setRoleFormRuntime(props.activeSession()!.selectedAssistant!);
            return next;
          })}
          class={`min-h-7 rounded-md border border-white/[0.08] px-2 py-0.5 text-xs text-zinc-500 hover:border-white/[0.15] hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
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

      <div class="space-y-1">
        <For each={props.roles()}>
          {(role) => {
            const cfgKeyCount = createMemo(() => {
              try {
                const c = JSON.parse(role.configOptionsJson || "{}");
                return Object.keys(c).filter((k) => k !== "model" && k !== "mode" && c[k]).length;
              } catch { return 0; }
            });
            return (
              <div class="group relative flex items-start gap-1.5 rounded-lg border-l-2 border-l-transparent px-2 py-1.5 motion-safe:transition-all motion-safe:duration-150 hover:border-l-white/20 hover:bg-white/[0.04]">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-1.5 flex-wrap">
                    <span class="text-xs font-medium text-zinc-300">{role.roleName}</span>
                    <span class={`text-xs ${RUNTIME_COLOR[role.runtimeKind] ?? "text-zinc-500"}`}>{role.runtimeKind}</span>
                    <Show when={role.model}><span class="rounded bg-blue-500/20 px-1 text-[9px] text-blue-200">{role.model}</span></Show>
                    <Show when={role.mode}><span class="rounded bg-indigo-500/20 px-1 text-[9px] text-indigo-200">{role.mode}</span></Show>
                    <Show when={!role.autoApprove}><span class="rounded bg-amber-500/20 px-1 text-[9px] text-amber-200">manual</span></Show>
                    <Show when={cfgKeyCount() > 0}><span class="rounded bg-teal-500/20 px-1 text-[9px] text-teal-200">{cfgKeyCount()} cfg</span></Show>
                  </div>
                  <p class="truncate text-xs text-zinc-600">{role.systemPrompt}</p>
                </div>
                <div class="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100">
                  <button
                    onClick={() => props.openRoleEditor(role)}
                    class={`rounded px-1.5 py-0.5 text-xs ${props.editingRoleId() === role.id ? "text-blue-300" : "text-zinc-500 hover:text-zinc-200"} ${INTERACTIVE_MOTION}`}
                  >
                    {props.editingRoleId() === role.id ? "editing" : "edit"}
                  </button>
                </div>
              </div>
            );
          }}
        </For>
        <Show when={props.roles().length === 0 && !props.showRoleForm()}>
          <p class="rounded-lg border border-dashed border-zinc-700 p-2 text-center text-xs text-zinc-600">
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
  fetchConfigOptions: (runtimeKey: string, roleName?: string, teamId?: string | null) => Promise<AcpConfigOption[]>;
};

function CreateRoleForm(props: CreateRoleFormProps) {
  return (
    <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
      <input
        value={props.roleFormName()}
        onInput={(e) => props.setRoleFormName(e.currentTarget.value)}
        placeholder="Role name (e.g. Developer)"
        class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
      />
      <select
        value={props.roleFormRuntime()}
        onChange={(e) => {
          props.setRoleFormRuntime(e.currentTarget.value);
          props.setRoleFormConfigSelections({});
          void props.fetchConfigOptions(e.currentTarget.value).then(props.setCreateFormConfigOptions);
        }}
        class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
      >
        <For each={RUNTIMES}>{(r) => <option value={r}>{r}</option>}</For>
      </select>
      <textarea
        value={props.roleFormPrompt()}
        onInput={(e) => props.setRoleFormPrompt(e.currentTarget.value)}
        rows={3}
        placeholder="System prompt for this role..."
        class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
      />
      <For each={props.createFormConfigOptions()}>
        {(opt) => {
          const values = () => flattenConfigValues(opt.options);
          const selected = () => props.roleFormConfigSelections()[opt.id] ?? "";
          const isModel = () => opt.category === "model" || opt.id === "model";
          const isMode = () => opt.category === "mode" || opt.id === "mode";
          return (
            <Show when={!isModel() && !isMode()}>
              <div class="flex flex-col gap-0.5">
                <label class="text-[10px] text-zinc-500">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
                <select
                  value={selected()}
                  onChange={(e) => props.setRoleFormConfigSelections((prev) => ({ ...prev, [opt.id]: e.currentTarget.value }))}
                  class="h-8 w-full rounded border border-zinc-700 bg-zinc-900 px-2 text-xs text-zinc-200"
                >
                  <option value="">(default: {opt.currentValue})</option>
                  <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
                </select>
              </div>
            </Show>
          );
        }}
      </For>
      <Show when={props.createFormConfigOptions().some((o) => o.category === "model" || o.id === "model")}>
        {(() => {
          const modelOpt = () => props.createFormConfigOptions().find((o) => o.category === "model" || o.id === "model")!;
          const values = () => flattenConfigValues(modelOpt().options);
          return (
            <select
              value={props.roleFormModel()}
              onChange={(e) => props.setRoleFormModel(e.currentTarget.value)}
              class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
            >
              <option value="">Model (default: {modelOpt().currentValue})</option>
              <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
            </select>
          );
        })()}
      </Show>
      <Show when={!props.createFormConfigOptions().some((o) => o.category === "model" || o.id === "model")}>
        <input
          value={props.roleFormModel()}
          onInput={(e) => props.setRoleFormModel(e.currentTarget.value)}
          placeholder="Model (optional)"
          class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
        />
      </Show>
      <Show when={props.createFormConfigOptions().some((o) => o.category === "mode" || o.id === "mode")}>
        {(() => {
          const modeOpt = () => props.createFormConfigOptions().find((o) => o.category === "mode" || o.id === "mode")!;
          const values = () => flattenConfigValues(modeOpt().options);
          return (
            <select
              value={props.roleFormMode()}
              onChange={(e) => props.setRoleFormMode(e.currentTarget.value)}
              class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
            >
              <option value="">Mode (default: {modeOpt().currentValue})</option>
              <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
            </select>
          );
        })()}
      </Show>
      <Show when={!props.createFormConfigOptions().some((o) => o.category === "mode" || o.id === "mode")}>
        <select
          value={props.roleFormMode()}
          onChange={(e) => props.setRoleFormMode(e.currentTarget.value)}
          class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
        >
          <option value="">Mode (default)</option>
          <option value="plan">plan</option>
          <option value="act">act</option>
          <option value="auto">auto</option>
        </select>
      </Show>
      <label class="flex items-center gap-2 text-xs text-zinc-500">
        <input
          type="checkbox"
          checked={props.roleFormAutoApprove()}
          onChange={(e) => props.setRoleFormAutoApprove(e.currentTarget.checked)}
          class="rounded"
        />
        Auto-approve permissions
      </label>
      <button
        onClick={() => void props.handleCreateRole()}
        class={`h-9 w-full rounded-lg bg-white text-sm font-semibold text-zinc-950 ${INTERACTIVE_MOTION}`}
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
    <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
      <div class="flex items-center justify-between">
        <div class="text-xs font-semibold text-zinc-200">Edit {props.role().roleName}</div>
        <button
          onClick={props.closeRoleEditor}
          class={`rounded px-1.5 py-0.5 text-xs text-zinc-500 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
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
        class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
      />
      <Show when={modelOpt()} fallback={
        <input value={props.editRoleModel()} onInput={(e) => props.setEditRoleModel(e.currentTarget.value)} placeholder="Model (optional)" class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200" />
      }>
        {(mo) => {
          const values = () => flattenConfigValues(mo().options);
          return (
            <select value={props.editRoleModel() || mo().currentValue} onChange={(e) => props.setEditRoleModel(e.currentTarget.value)} class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200">
              <option value="">Model (default: {mo().currentValue})</option>
              <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
            </select>
          );
        }}
      </Show>
      <Show when={modeOpt()} fallback={
        <input value={props.editRoleMode()} onInput={(e) => props.setEditRoleMode(e.currentTarget.value)} placeholder="Mode (optional)" class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200" />
      }>
        {(mo) => {
          const values = () => flattenConfigValues(mo().options);
          return (
            <select value={props.editRoleMode() || mo().currentValue} onChange={(e) => props.setEditRoleMode(e.currentTarget.value)} class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200">
              <option value="">Mode (default: {mo().currentValue})</option>
              <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
            </select>
          );
        }}
      </Show>
      <For each={otherOpts()}>
        {(opt) => {
          const values = () => flattenConfigValues(opt.options);
          const selected = () => currentCfg()[opt.id] ?? "";
          return (
            <div class="flex flex-col gap-0.5">
              <label class="text-[10px] text-zinc-500">{opt.name}{opt.description ? ` — ${opt.description}` : ""}</label>
              <select value={selected()} onChange={(e) => updateCfg(opt.id, e.currentTarget.value)} class="h-8 w-full rounded border border-zinc-700 bg-zinc-900 px-2 text-xs text-zinc-200">
                <option value="">(default: {opt.currentValue})</option>
                <For each={values()}>{(v) => <option value={v.value}>{v.name}{v.description ? ` — ${v.description}` : ""}</option>}</For>
              </select>
            </div>
          );
        }}
      </For>
      <Show when={opts().length === 0}>
        <div class="text-[10px] text-zinc-600 italic">
          {props.activeSession()?.configOptionsLoading ? "Loading config options from agent..." : "No config options available for this agent."}
        </div>
      </Show>
      <textarea
        value={props.editRoleMcpServersJson()}
        onInput={(e) => props.setEditRoleMcpServersJson(e.currentTarget.value)}
        rows={2}
        placeholder='MCP servers JSON (e.g. [{"name":"..." }])'
        class="w-full resize-y rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-xs font-mono text-zinc-300"
      />
      <label class="flex items-center gap-2 text-xs text-zinc-500">
        <input
          type="checkbox"
          checked={props.editRoleAutoApprove()}
          onChange={(e) => props.setEditRoleAutoApprove(e.currentTarget.checked)}
          class="rounded"
        />
        Auto-approve permissions
      </label>
      <button
        onClick={() => void props.handleSaveRoleEdit()}
        class={`h-9 w-full rounded-lg bg-white text-sm font-semibold text-zinc-950 ${INTERACTIVE_MOTION}`}
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
        <span class="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Skills</span>
        <button
          onClick={() => {
            props.setEditingSkillId(null);
            props.setSkillFormName("");
            props.setSkillFormDescription("");
            props.setSkillFormContent("");
            props.setShowSkillForm((v) => !v);
          }}
          class={`min-h-7 rounded-md border border-white/[0.08] px-2 py-0.5 text-xs text-zinc-500 hover:border-white/[0.15] hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
        >
          {props.showSkillForm() && !props.editingSkillId() ? "cancel" : "+ skill"}
        </button>
      </div>

      <Show when={props.showSkillForm()}>
        <div class="mb-2 space-y-1.5 rounded-xl border border-white/[0.07] bg-white/[0.03] p-2.5">
          <input
            value={props.skillFormName()}
            onInput={(e) => props.setSkillFormName(e.currentTarget.value)}
            placeholder="Skill name (e.g. code-review)"
            class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
          />
          <input
            value={props.skillFormDescription()}
            onInput={(e) => props.setSkillFormDescription(e.currentTarget.value)}
            placeholder="Short description"
            class="h-9 w-full rounded border border-zinc-700 bg-zinc-900 px-2.5 text-sm text-zinc-200"
          />
          <textarea
            value={props.skillFormContent()}
            onInput={(e) => props.setSkillFormContent(e.currentTarget.value)}
            rows={4}
            placeholder="Skill content - prompt instructions, templates, context..."
            class="w-full resize-none rounded border border-zinc-700 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200"
          />
          <button
            onClick={props.saveSkill}
            class={`w-full rounded-lg bg-indigo-600 py-1.5 text-xs font-medium text-white hover:bg-indigo-500 ${INTERACTIVE_MOTION}`}
          >
            {props.editingSkillId() ? "Save changes" : "Create skill"}
          </button>
        </div>
      </Show>

      <div class="space-y-1">
        <For each={props.skills()}>
          {(skill) => (
            <div class="group rounded-lg border border-white/[0.05] bg-white/[0.02] px-2.5 py-2">
              <div class="flex items-center justify-between gap-1">
                <span class="text-xs font-medium text-zinc-300">#{skill.name}</span>
                <div class="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={() => props.openSkillEditor(skill)}
                    class="rounded px-1.5 py-0.5 text-[10px] text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.06]"
                  >edit</button>
                  <button
                    onClick={() => void props.deleteSkill(skill.id)}
                    class="rounded px-1.5 py-0.5 text-[10px] text-rose-600 hover:text-rose-400 hover:bg-rose-500/10"
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
          <p class="rounded-lg border border-dashed border-zinc-700 p-2 text-center text-xs text-zinc-600">
            No skills yet. Click "+ skill" to add one.
          </p>
        </Show>
      </div>
    </div>
  );
}
