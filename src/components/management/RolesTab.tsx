import { For, Show, createEffect, createMemo, createSignal, onMount, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import { Copy, Edit3, Files, FolderGit2, Globe, Trash2 } from "lucide-solid";
import type { AppSession, Role, RoleUpsertInput, AcpConfigOption, AssistantRuntime, Project } from "../types";
import { RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "../types";
import { EmptyState, FieldRow, TextInput, InlineSelect, ActionButton } from "./primitives";
import { roleApi, assistantApi, globalMcpApi, ruleApi, skillApi, parseError } from "../../lib/tauriApi";
import type { RoleMcpEntry, RoleRule, RoleSkill } from "../../lib/tauriApi";
import { isModeOption, isModelOption, optionCurrentValue } from "../../lib/configOptions";
import {
  appliesToModel,
  effectiveDefault,
  findModelOption,
  otherRuntimeOptions,
  readRuntimeOptions,
  storedValue,
  valuesForModel,
  type RuntimeOption,
} from "../../lib/runtimeOptions";
import { ContextMenuSurface, ContextMenuItem, ContextMenuSeparator } from "../ui";

function ToggleField(props: { label: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <FieldRow label={props.label}>
      <label class="flex items-center gap-2 text-[11px] theme-muted">
        <input
          type="checkbox"
          checked={props.on}
          onChange={(e) => props.onChange(e.currentTarget.checked)}
        />
        <span>{props.on ? "On" : "Off"}</span>
      </label>
    </FieldRow>
  );
}

/**
 * Renders whatever parameters a runtime declared, other than the two this editor gives their
 * own rows (model, and mode where a runtime exposes a separate mode list). A select becomes a
 * dropdown of its declared values; a toggle becomes a checkbox. Previously this file knew the
 * names "model", "mode" and "effort" and rendered anything else as a select — which is why a
 * boolean knob appeared here as an empty dropdown while the composer showed a working switch.
 */
function ParameterFields(props: {
  declared: RuntimeOption[];
  modelId: string;
  value: (id: string) => string;
  onChange: (id: string, value: string) => void;
}) {
  const modelEntry = () =>
    props.declared
      .find((o) => o.id === "model")
      ?.values.find((v) => v.value === props.modelId);
  const visible = () =>
    otherRuntimeOptions(props.declared)
      .filter((o) => o.id !== "mode")
      .filter((o) => appliesToModel(o, props.modelId));

  return (
    <For each={visible()}>
      {(option) => {
        if (option.kind === "toggle") {
          return (
            <ToggleField
              label={option.name}
              on={props.value(option.id) === "true"}
              onChange={(on) => props.onChange(option.id, on ? "true" : "")}
            />
          );
        }
        const choices = () => valuesForModel(option, modelEntry());
        const fallback = () => effectiveDefault(option, modelEntry()) || "runtime";
        return (
          <Show when={choices().length > 0}>
            <FieldRow label={option.name}>
              <InlineSelect
                value={props.value(option.id)}
                options={[
                  { value: "", label: `default: ${fallback()}` },
                  ...choices().map((v) => ({
                    value: v.value,
                    label: v.description ? `${v.name} — ${v.description}` : v.name,
                  })),
                ]}
                onChange={(val) => props.onChange(option.id, val)}
              />
            </FieldRow>
          </Show>
        );
      }}
    </For>
  );
}


const CAPABILITY_CHIPS: Array<[string, string]> = [
  ["mcpServers", "MCP"],
  ["modelCatalog", "models"],
  ["dynamicModes", "modes"],
  ["toolDetail", "tool detail"],
  ["outputStreaming", "output stream"],
  ["usage", "usage"],
  ["nestedTools", "subagents"],
  ["rewind", "rewind"],
  ["fork", "fork"],
];

/** `interaction` is tri-state, so it gets its own chip rather than a boolean. */
const INTERACTION_LABEL: Record<string, string> = {
  none: "no prompts",
  permissionOnly: "permissions",
  full: "permissions + questions",
};

function CapabilityChips(props: { assistant: AssistantRuntime | undefined }) {
  return (
    <Show when={props.assistant}>
      {(assistant) => {
        const caps = () => assistant().capabilities || {};
        return (
          <div class="mt-1 flex flex-wrap gap-1">
            <For each={CAPABILITY_CHIPS}>
              {([key, label]) => (
                <span
                  title={caps()[key] ? `${label}: supported` : `${label}: not supported by this provider`}
                  class={`rounded border px-1.5 py-0.5 font-mono text-[9px] ${
                    caps()[key]
                      ? "border-[var(--ui-border)] theme-text"
                      : "border-transparent theme-muted opacity-40 line-through"
                  }`}
                >
                  {label}
                </span>
              )}
            </For>
            <Show when={typeof caps().interaction === "string"}>
              <span
                title="How much of the agent-asks-the-user surface this provider exposes"
                class="rounded border px-1.5 py-0.5 font-mono text-[9px]"
                classList={{
                  "border-transparent theme-muted opacity-40": caps().interaction === "none",
                  "border-[var(--ui-border)] theme-text": caps().interaction !== "none",
                }}
              >
                {INTERACTION_LABEL[caps().interaction as string] ?? String(caps().interaction)}
              </span>
            </Show>
          </div>
        );
      }}
    </Show>
  );
}

export function RolesTab(props: {
  assistants: Accessor<AssistantRuntime[]>;
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  refreshRoles: (projectId?: string) => Promise<void>;
  fetchRoleConfig: (runtimeKey: string, roleName?: string, forceRefresh?: boolean) => Promise<{ options: AcpConfigOption[]; modes: string[] }>;
  pushMessage: (role: string, text: string) => void;
  initialRoleName?: string;
  currentProject?: Accessor<Project | null>;
}) {
  const userRoles = createMemo(() => props.roles());

  // "creating" = create form open; selectedId = which role is being edited
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [deletingId, setDeletingId] = createSignal<string | null>(null);
  const [deleteError, setDeleteError] = createSignal<string | null>(null);
  const [deletingRole, setDeletingRole] = createSignal<string | null>(null);
  const [roleContextMenu, setRoleContextMenu] = createSignal<{ x: number; y: number; role: Role } | null>(null);

  const closeRoleContextMenu = () => setRoleContextMenu(null);

  onMount(() => {
    const handleGlobalClick = () => closeRoleContextMenu();
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeRoleContextMenu();
    };
    window.addEventListener("pointerdown", handleGlobalClick);
    window.addEventListener("keydown", handleKeyDown);
    onCleanup(() => {
      window.removeEventListener("pointerdown", handleGlobalClick);
      window.removeEventListener("keydown", handleKeyDown);
    });
  });

  const copyRoleAssociations = async (sourceRoleId: string, targetRoleId: string) => {
    try {
      const [mcpList, ruleList, skillList] = await Promise.all([
        globalMcpApi.listRoleMcp(sourceRoleId).catch(() => []),
        ruleApi.listRoleRules(sourceRoleId).catch(() => []),
        skillApi.listAllSkillsForRole(sourceRoleId).catch(() => []),
      ]);

      const mcpPromises = mcpList
        .filter((m) => m.enabled)
        .map((m) => globalMcpApi.setRoleMcpEnabled(targetRoleId, m.mcpServerName, true).catch(() => {}));

      const rulePromise = ruleList.length > 0
        ? ruleApi.setRoleRules(targetRoleId, ruleList.map((r) => [r.ruleId, r.enabled, r.ord])).catch(() => {})
        : Promise.resolve();

      const skillPromise = skillList.length > 0
        ? skillApi.setRoleSkills(targetRoleId, skillList.map((s) => [s.skillId, s.enabled, s.ord])).catch(() => {})
        : Promise.resolve();

      await Promise.all([...mcpPromises, rulePromise, skillPromise]);
    } catch {
      // Best effort association copy
    }
  };

  const duplicateRole = async (role: Role) => {
    const newName = uniqueRoleName(`${role.roleName}_copy`, role.projectId ?? null);
    try {
      setSaving(true);
      const created = await roleApi.upsert({
        roleName: newName,
        runtimeKind: role.runtimeKind,
        runtimeProfileId: role.runtimeProfileId,
        systemPrompt: role.systemPrompt,
        model: role.model,
        mode: role.mode,
        mcpServersJson: role.mcpServersJson,
        configOptionsJson: role.configOptionsJson,
        configOptionDefsJson: role.configOptionDefsJson,
        autoApprove: role.autoApprove,
        projectId: role.projectId,
      });
      await copyRoleAssociations(role.id, created.id);
      await props.refreshRoles(props.currentProject?.()?.id);
      openEdit(created);
      props.pushMessage("event", `Role duplicated as '${newName}'`);
    } catch (e) {
      const err = parseError(e);
      props.pushMessage("event", `Failed to duplicate role: ${err.message}`);
    } finally {
      setSaving(false);
    }
  };

  const copyRoleToProject = async (role: Role, targetProject: Project) => {
    const targetName = uniqueRoleName(role.roleName, targetProject.id);
    try {
      setSaving(true);
      const created = await roleApi.upsert({
        roleName: targetName,
        runtimeKind: role.runtimeKind,
        runtimeProfileId: role.runtimeProfileId,
        systemPrompt: role.systemPrompt,
        model: role.model,
        mode: role.mode,
        mcpServersJson: role.mcpServersJson,
        configOptionsJson: role.configOptionsJson,
        configOptionDefsJson: role.configOptionDefsJson,
        autoApprove: role.autoApprove,
        projectId: targetProject.id,
      });
      await copyRoleAssociations(role.id, created.id);
      await props.refreshRoles(targetProject.id);
      openEdit(created);
      props.pushMessage("event", `Role '${role.roleName}' copied to project '${targetProject.name}' as '${targetName}'`);
    } catch (e) {
      const err = parseError(e);
      props.pushMessage("event", `Failed to copy role to project: ${err.message}`);
    } finally {
      setSaving(false);
    }
  };

  // Scope & filter states
  const [cScope, setCScope] = createSignal<"project" | "global">(props.currentProject?.() ? "project" : "global");
  const [eScope, setEScope] = createSignal<"project" | "global">("global");
  const [roleFilter, setRoleFilter] = createSignal<"all" | "project" | "global">("all");

  const projectRoles = createMemo(() => {
    const currentProjId = props.currentProject?.()?.id;
    if (!currentProjId) return [];
    return userRoles().filter((r) => r.projectId === currentProjId);
  });

  const globalRoles = createMemo(() => {
    return userRoles().filter((r) => !r.projectId);
  });

  const filteredRoles = createMemo(() => {
    const filter = roleFilter();
    if (filter === "project") return projectRoles();
    if (filter === "global") return globalRoles();
    return [...projectRoles(), ...globalRoles()];
  });

  // ── Create form state ───────────────────────────────────────────────────────
  const [cName, setCName] = createSignal("Developer");
  const [cRuntime, setCRuntime] = createSignal("claude-native");

  // New roles default to the native Claude provider; fall back to ACP Claude
  // when the native CLI is not detected on this machine.
  createEffect(() => {
    if (cRuntime() !== "claude-native") return;
    const assistants = props.assistants();
    const native = assistants.find((a) => a.profileId === "native:claude");
    const acpClaude = assistants.find((a) => a.profileId === "acp:claude-code");
    if (native && !native.available && acpClaude?.available) {
      setCRuntime(acpClaude.key);
    }
  });
  const [cPrompt, setCPrompt] = createSignal("");
  const [cModel, setCModel] = createSignal("");
  const [cMode, setCMode] = createSignal("");
  const [cConfigOpts, setCConfigOpts] = createSignal<AcpConfigOption[]>([]);
  const [cConfigSel, setCConfigSel] = createSignal<Record<string, string>>({});
  const [cConfigLoading, setCConfigLoading] = createSignal(false);
  const [cModes, setCModes] = createSignal<string[]>([]);
  const [cGlobalMcp, setCGlobalMcp] = createSignal<RoleMcpEntry[]>([]);
  const [cRoleRules, setCRoleRules] = createSignal<RoleRule[]>([]);
  const [cRoleSkills, setCRoleSkills] = createSignal<RoleSkill[]>([]);

  // ── Edit form state ─────────────────────────────────────────────────────────
  const [ePrompt, setEPrompt] = createSignal("");
  const [eModel, setEModel] = createSignal("");
  const [eMode, setEMode] = createSignal("");
  const [eCfgJson, setECfgJson] = createSignal("{}");
  const [eConfigOpts, setEConfigOpts] = createSignal<AcpConfigOption[]>([]);
  const [eConfigLoading, setEConfigLoading] = createSignal(false);
  const [eModes, setEModes] = createSignal<string[]>([]);
  const [eGlobalMcp, setEGlobalMcp] = createSignal<RoleMcpEntry[]>([]);
  const [mcpResetting, setMcpResetting] = createSignal(false);
  const [eRoleRules, setERoleRules] = createSignal<RoleRule[]>([]);
  const [eRoleSkills, setERoleSkills] = createSignal<RoleSkill[]>([]);
  const [roleConfigCache, setRoleConfigCache] = createSignal<Record<string, { options: AcpConfigOption[]; modes: string[] }>>({});
  let createConfigReqSeq = 0;
  let editConfigReqSeq = 0;

  const editingRole = createMemo(() =>
    selectedId() ? userRoles().find((r) => r.id === selectedId()) ?? null : null,
  );

  const loadCreateBindings = () => {
    void globalMcpApi.listRoleMcp("__new_role__").then(setCGlobalMcp).catch(() => setCGlobalMcp([]));
    void ruleApi.listAllRulesForRole("__new_role__").then(setCRoleRules).catch(() => setCRoleRules([]));
    void skillApi.listAllSkillsForRole("__new_role__").then(setCRoleSkills).catch(() => setCRoleSkills([]));
  };

  // ── Open create ─────────────────────────────────────────────────────────────
  const openCreate = () => {
    const nativeClaude = props.assistants().find((assistant) => assistant.profileId === "native:claude");
    const acpClaude = props.assistants().find((assistant) => assistant.profileId === "acp:claude-code");
    const defaultRuntime = nativeClaude && !nativeClaude.available && acpClaude?.available
      ? acpClaude.key
      : "claude-native";
    const reqSeq = ++createConfigReqSeq;
    setCName("Developer");
    setCRuntime(defaultRuntime);
    setCPrompt("");
    setCModel("");
    setCMode("");
    setCConfigOpts([]);
    setCConfigSel({});
    setCGlobalMcp([]);
    setCRoleRules([]);
    setCRoleSkills([]);
    setSelectedId(null);
    setDeletingId(null);
    setDeleteError(null);
    setCreating(true);
    setCScope(props.currentProject?.() ? "project" : "global");
    setCModes([]);
    setCConfigLoading(true);
    void props.fetchRoleConfig(`runtime:${defaultRuntime}`, undefined, true)
      .then(({ options, modes }) => {
        if (reqSeq !== createConfigReqSeq || !creating() || cRuntime() !== defaultRuntime) return;
        setCConfigOpts(options);
        setCModes(modes);
      })
      .catch(() => {
        if (reqSeq !== createConfigReqSeq) return;
        setCConfigOpts([]);
        setCModes([]);
      })
      .finally(() => {
        if (reqSeq === createConfigReqSeq) setCConfigLoading(false);
      });
    loadCreateBindings();
  };

  // ── Open edit ───────────────────────────────────────────────────────────────
  const openEdit = (role: Role) => {
    const reqSeq = ++editConfigReqSeq;
    setCreating(false);
    setDeleteError(null);
    setSelectedId(role.id);
    setEScope(role.projectId ? "project" : "global");
    setEPrompt(role.systemPrompt ?? "");
    setEModel(role.model ?? "");
    setEMode(role.mode ?? "");
    setECfgJson(role.configOptionsJson || "{}");
    setEConfigOpts([]);
    setEModes([]);
    setEConfigLoading(true);
    setERoleRules([]);
    setERoleSkills([]);
    void props.fetchRoleConfig(role.runtimeKind, role.roleName, true)
      .then(({ options, modes }) => {
        if (reqSeq !== editConfigReqSeq || creating() || selectedId() !== role.id) return;
        setEConfigOpts(options);
        setEModes(modes);
        setRoleConfigCache((prev) => ({ ...prev, [role.roleName]: { options, modes } }));
      })
      .catch(() => {
        if (reqSeq !== editConfigReqSeq) return;
        setEConfigOpts([]);
        setEModes([]);
      })
      .finally(() => {
        if (reqSeq === editConfigReqSeq) setEConfigLoading(false);
      });
    void globalMcpApi.listRoleMcp(role.id).then(setEGlobalMcp).catch(() => {});
    void ruleApi.listAllRulesForRole(role.id).then(setERoleRules).catch(() => {});
    void skillApi.listAllSkillsForRole(role.id).then(setERoleSkills).catch(() => {});
  };

  const refreshEditConfig = () => {
    const role = editingRole();
    if (!role || eConfigLoading()) return;
    const reqSeq = ++editConfigReqSeq;
    setEConfigLoading(true);
    void props.fetchRoleConfig(role.runtimeKind, role.roleName, true)
      .then(({ options, modes }) => {
        if (reqSeq !== editConfigReqSeq || creating() || selectedId() !== role.id) return;
        setEConfigOpts(options);
        setEModes(modes);
        setRoleConfigCache((prev) => ({ ...prev, [role.roleName]: { options, modes } }));
      })
      .catch(() => {})
      .finally(() => {
        if (reqSeq === editConfigReqSeq) setEConfigLoading(false);
      });
  };

  // Auto-open edit when panel is opened from sidebar with a pre-selected role name.
  // Guard: only run once (when selectedId is still null and creating is false).
  createEffect(() => {
    if (!props.initialRoleName || creating() || selectedId()) return;
    const role = userRoles().find((r) => r.roleName === props.initialRoleName);
    if (role) openEdit(role);
  });

  // ── Helpers ─────────────────────────────────────────────────────────────────
  const uniqueRoleName = (desired: string, targetProjectId?: string | null): string => {
    const pool = targetProjectId === undefined
      ? userRoles()
      : userRoles().filter((r) => (targetProjectId ? r.projectId === targetProjectId : !r.projectId));
    const existing = new Set(pool.map((r) => r.roleName.toLowerCase()));
    if (!existing.has(desired.toLowerCase())) return desired;
    const base = desired.replace(/_copy(\d+)?$/, "");
    let n = 2;
    let candidate = `${base}_copy`;
    while (existing.has(candidate.toLowerCase())) candidate = `${base}_copy${n++}`;
    return candidate;
  };

  const resolvedModes = (configOpts: AcpConfigOption[], modesArr: string[]) => {
    if (modesArr.length > 0) return { kind: "list" as const, modes: modesArr };
    const modeOpt = configOpts.find(isModeOption);
    if (modeOpt) return { kind: "option" as const, opt: modeOpt };
    return null;
  };

  const parseRoleConfigMap = (json?: string | null): Record<string, string> => {
    try {
      const parsed = JSON.parse(json || "{}");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
      return Object.fromEntries(
        Object.entries(parsed)
          .filter(([, value]) => typeof value === "string" && value.trim().length > 0)
          .map(([key, value]) => [key, value as string]),
      );
    } catch {
      return {};
    }
  };

  const parseRoleConfigDefs = (json?: string | null): AcpConfigOption[] => {
    try {
      const parsed = JSON.parse(json || "[]");
      return Array.isArray(parsed) ? (parsed as AcpConfigOption[]) : [];
    } catch {
      return [];
    }
  };

  const roleConfigDefs = (role: Role): AcpConfigOption[] => {
    const cached = roleConfigCache()[role.roleName]?.options;
    if (cached?.length) return cached;
    return parseRoleConfigDefs(role.configOptionDefsJson);
  };

  /** The chip shown beside a persona in the list: its first configured select parameter. The
   *  runtime states which parameters exist, so this no longer branches on `codex-cli` or
   *  guesses among four historical key spellings. */
  const roleEffort = (role: Role) => {
    const cfg = parseRoleConfigMap(role.configOptionsJson);
    const declared = readRuntimeOptions(roleConfigDefs(role));
    const modelId = role.model || cfg.model || "";
    const model = findModelOption(declared)?.values.find((v) => v.value === modelId);
    for (const option of otherRuntimeOptions(declared)) {
      if (option.kind !== "select" || !appliesToModel(option, modelId)) continue;
      const value = storedValue(cfg, option) || effectiveDefault(option, model);
      if (value) return value;
    }
    return null;
  };

  const roleMode = (role: Role) => {
    if (role.mode) return role.mode;
    const cfg = parseRoleConfigMap(role.configOptionsJson);
    if (cfg.mode) return cfg.mode;
    const defs = roleConfigDefs(role);
    const opt = defs.find(isModeOption);
    return opt ? optionCurrentValue(opt) || null : null;
  };

  const roleModel = (role: Role) => {
    if (role.model) return role.model;
    const cfg = parseRoleConfigMap(role.configOptionsJson);
    if (cfg.model) return cfg.model;
    const defs = roleConfigDefs(role);
    const opt = defs.find(isModelOption);
    return opt ? optionCurrentValue(opt) || null : null;
  };

  const supportsCapability = (runtimeKey: string, capability: string) =>
    props.assistants().find((assistant) => assistant.key === runtimeKey)?.capabilities[capability] === true;

  const configOptionSelectOptions = (opt: AcpConfigOption, defaultLabel = "default") =>
    [{ value: "", label: `${defaultLabel}: ${optionCurrentValue(opt) || "runtime"}` }, ...flattenConfigValues(opt.options).map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))];

  // Last-resort list, shown only before the backend catalog has been discovered for this
  // runtime. Prefer family aliases over pinned ids here: the CLI resolves an alias to
  // whatever the installed build treats as current, so these cannot go stale.
  const RUNTIME_DEFAULT_MODELS: Record<string, { value: string; label: string }[]> = {
    "claude-native": [
      { value: "", label: "default: runtime" },
      { value: "opus", label: "Opus (alias)" },
      { value: "sonnet", label: "Sonnet (alias)" },
      { value: "haiku", label: "Haiku (alias)" },
      { value: "fable", label: "Fable (alias)" },
    ],
    "claude-code": [
      { value: "", label: "default: runtime" },
      { value: "opus", label: "Opus (alias)" },
      { value: "sonnet", label: "Sonnet (alias)" },
      { value: "haiku", label: "Haiku (alias)" },
      { value: "fable", label: "Fable (alias)" },
    ],
    "antigravity-cli": [{ value: "", label: "default: runtime" }],
    "codex-cli": [{ value: "", label: "default: runtime" }],
  };

  const isProjectOverride = (role: Role) =>
    Boolean(role.projectId && globalRoles().some((gr) => gr.roleName.toLowerCase() === role.roleName.toLowerCase()));

  const resolvedModelOptions = (runtimeKind: string, modelOpt?: AcpConfigOption) => {
    // The catalog is scoped per-runtime in the backend, so whatever arrives here already
    // belongs to this runtime — no client-side family filtering.
    if (modelOpt && modelOpt.options && modelOpt.options.length > 0) {
      return configOptionSelectOptions(modelOpt);
    }

    for (const [key, list] of Object.entries(RUNTIME_DEFAULT_MODELS)) {
      if (runtimeKind.includes(key) || key.includes(runtimeKind)) {
        return list;
      }
    }
    return [{ value: "", label: "default: runtime" }];
  };

  // ── Create submit ───────────────────────────────────────────────────────────
  const handleCreate = async () => {
    const targetPid = cScope() === "project" ? (props.currentProject?.()?.id ?? null) : null;
    const rawName = cName().trim();
    if (!rawName || saving()) return;
    if (/\s/.test(rawName)) { props.pushMessage("event", "Role name cannot contain spaces."); return; }
    if (!/^[A-Za-z0-9_-]+$/.test(rawName)) { props.pushMessage("event", "Role name only allows letters, numbers, - and _."); return; }
    const name = uniqueRoleName(rawName, targetPid);
    setSaving(true);
    const configMap: Record<string, string> = {};
    for (const [k, v] of Object.entries(cConfigSel())) { if (v) configMap[k] = v; }
    try {
      const saved = await roleApi.upsert({
        roleName: name, runtimeKind: cRuntime(), runtimeProfileId: selectedAssistant()?.profileId,
        systemPrompt: cPrompt().trim(),
        model: cModel().trim() || null, mode: cMode().trim() || null,
        mcpServersJson: "[]", configOptionsJson: JSON.stringify(configMap),
        configOptionDefsJson: JSON.stringify(resolvedCreateConfigOpts()),
        autoApprove: true,
        projectId: targetPid,
      } satisfies RoleUpsertInput);
      if (supportsCapability(cRuntime(), "mcpServers")) {
        await Promise.all(cGlobalMcp().map((entry) =>
          globalMcpApi.setRoleMcpEnabled(saved.id, entry.mcpServerName, entry.enabled).catch(() => {}),
        ));
      }
      await ruleApi.setRoleRules(
        saved.id,
        cRoleRules().map((r) => [r.ruleId, r.enabled, r.ord] as [string, boolean, number]),
      ).catch(() => {});
      await skillApi.setRoleSkills(
        saved.id,
        cRoleSkills().map((s) => [s.skillId, s.enabled, s.ord] as [string, boolean, number]),
      ).catch(() => {});
      await props.refreshRoles(props.currentProject?.()?.id);
      openEdit(saved);
      setCConfigSel({}); setCConfigOpts([]); setCGlobalMcp([]); setCRoleRules([]); setCRoleSkills([]);
      props.pushMessage("event", `Role created: ${saved.roleName} (${saved.projectId ? "project" : "global"})`);
    } catch (e) { const err = parseError(e); props.pushMessage("event", `Failed to create role: ${err.message}`); }
    finally { setSaving(false); }
  };

  // ── Edit submit ─────────────────────────────────────────────────────────────
  const handleSaveEdit = async () => {
    const role = editingRole();
    if (!role || saving()) return;
    let parsedCfg: unknown;
    try { parsedCfg = JSON.parse(eCfgJson().trim() || "{}"); if (!parsedCfg || typeof parsedCfg !== "object" || Array.isArray(parsedCfg)) throw new Error("must be object"); }
    catch (e) { props.pushMessage("event", `Invalid config JSON: ${String(e)}`); return; }
    const previousMode = role.mode ?? null;
    const newMode = eMode().trim() || null;
    const modeChanged = newMode !== previousMode;
    const targetPid = eScope() === "project" ? (role.projectId ?? props.currentProject?.()?.id ?? null) : null;
    setSaving(true);
    try {
      await roleApi.upsert({
        id: role.id,
        roleName: role.roleName, runtimeKind: role.runtimeKind, runtimeProfileId: role.runtimeProfileId,
        systemPrompt: ePrompt().trim(), model: eModel().trim() || null,
        mode: newMode, mcpServersJson: "[]",
        configOptionsJson: JSON.stringify(parsedCfg),
        configOptionDefsJson: JSON.stringify(resolvedEditConfigOpts()),
        autoApprove: true,
        projectId: targetPid,
      } satisfies RoleUpsertInput);
      await props.refreshRoles(props.currentProject?.()?.id);
      // Precedence rule: Session-specific overrides (`mode_override`) in the database take precedence over
      // the global role's default `mode`. When the global default mode changes, we sync it only to the
      // active sessions of this role that do not have any explicit session-level mode override.
      if (modeChanged && newMode) {
        const synced = await assistantApi.syncRoleMode(role.id, newMode).catch(() => []);
        for (const sessionId of synced) {
          props.updateSession(sessionId, { currentMode: newMode });
        }
      }
      props.pushMessage("event", `Role saved: ${role.roleName} (${targetPid ? "project" : "global"})`);
    } catch (e) { const err = parseError(e); props.pushMessage("event", `Failed to save: ${err.message}`); }
    finally { setSaving(false); }
  };

  // ── Delete ──────────────────────────────────────────────────────────────────
  const handleDelete = async (role: Role) => {
    if (deletingRole()) return;
    setDeleteError(null);
    setDeletingRole(role.id);
    const isOverride = isProjectOverride(role);
    try {
      await roleApi.remove(role.roleName, role.projectId ?? null, role.id);
      if (editingRole()?.id === role.id) setSelectedId(null);
      setDeletingId(null);
      await props.refreshRoles(props.currentProject?.()?.id);
      props.pushMessage("event", isOverride ? `Role '${role.roleName}' reverted to global` : `Role deleted: ${role.roleName}`);
    } catch (e) {
      const err = parseError(e);
      setDeleteError(err.message);
      props.pushMessage("event", `Failed to delete role: ${err.message}`);
    } finally {
      setDeletingRole(null);
    }
  };

  const handleToggleGlobalMcp = async (entry: RoleMcpEntry, enabled: boolean) => {
    const role = editingRole();
    if (!role || !supportsCapability(role.runtimeKind, "mcpServers")) return;
    await globalMcpApi.setRoleMcpEnabled(role.id, entry.mcpServerName, enabled).catch(() => {});
    setEGlobalMcp((prev) => prev.map((e) => e.mcpServerName === entry.mcpServerName ? { ...e, enabled } : e));
    setMcpResetting(true);
    await globalMcpApi.resetRoleMcpSessions(role.id).catch(() => {});
    setMcpResetting(false);
  };

  const handleToggleRule = async (rule: RoleRule, enabled: boolean) => {
    const role = editingRole();
    if (!role) return;
    const updated = eRoleRules().map((r) => r.ruleId === rule.ruleId ? { ...r, enabled } : r);
    setERoleRules(updated);
    const payload: [string, boolean, number][] = updated.map((r) => [r.ruleId, r.enabled, r.ord]);
    await ruleApi.setRoleRules(role.id, payload).catch(() => {});
  };

  const handleToggleSkill = async (skill: RoleSkill, enabled: boolean) => {
    const role = editingRole();
    if (!role) return;
    const updated = eRoleSkills().map((s) => s.skillId === skill.skillId ? { ...s, enabled } : s);
    setERoleSkills(updated);
    const payload: [string, boolean, number][] = updated.map((s) => [s.skillId, s.enabled, s.ord]);
    await skillApi.setRoleSkills(role.id, payload).catch(() => {});
  };

  const selectedAssistant = createMemo(() =>
    props.assistants().find((assistant) => assistant.key === cRuntime()),
  );

  const runtimeOptions = createMemo(() => {
    const nativeList: Array<{ value: string; label: string; group: string; disabled?: boolean; hint?: string }> = [];
    const acpList: Array<{ value: string; label: string; group: string; disabled?: boolean; hint?: string }> = [];
    const seen = new Set<string>();

    for (const assistant of props.assistants()) {
      seen.add(assistant.key);
      const isNative = assistant.family === "native";
      const item = {
        value: assistant.key,
        label: `${isNative ? "⚡ Native" : "🔌 ACP"} · ${assistant.label}`,
        group: isNative ? "⚡ Native CLI Engines (Top-Level Direct)" : "🔌 ACP Protocol Bridges (Drill-Down)",
        disabled: !assistant.available,
        hint: assistant.available
          ? `${isNative ? "Native CLI Direct Stream" : "ACP Protocol Bridge"} · ${assistant.transport}${assistant.launchMethod ? ` · ${assistant.launchMethod}` : ""}`
          : (assistant.installHint ?? "not detected on this machine"),
      };
      if (isNative) {
        nativeList.push(item);
      } else {
        acpList.push(item);
      }
    }

    for (const runtime of RUNTIMES) {
      if (!seen.has(runtime)) {
        const isNative = runtime.includes("native") || runtime === "antigravity-cli" || runtime === "codex-cli" || runtime === "pi-cli";
        const item = {
          value: runtime,
          label: `${isNative ? "⚡ Native" : "🔌 ACP"} · ${runtime}`,
          group: isNative ? "⚡ Native CLI Engines (Top-Level Direct)" : "🔌 ACP Protocol Bridges (Drill-Down)",
        };
        if (isNative) {
          nativeList.push(item);
        } else {
          acpList.push(item);
        }
      }
    }

    return [...nativeList, ...acpList];
  });

  // Edit form config options (local, not tied to global activeSession).
  // Codex used to get a hand-written reasoning-effort option bolted on here. It is now
  // discovered: `model/list` reports each model's own `supportedReasoningEfforts` (including
  // the `max` and `ultra` levels this list omitted, and without the `minimal` one it invented).
  const editConfigOpts = createMemo(() => eConfigOpts());
  const resolvedCreateConfigOpts = createMemo(() => cConfigOpts());
  const editCfgMap = createMemo((): Record<string, string> => {
    try { return JSON.parse(eCfgJson() || "{}"); } catch { return {}; }
  });
  const resolvedEditConfigOpts = createMemo(() => editConfigOpts());
  const updateEditCfg = (id: string, val: string) => {
    const map = { ...editCfgMap() };
    if (val) map[id] = val; else delete map[id];
    setECfgJson(JSON.stringify(map));
  };

  // ── Render ──────────────────────────────────────────────────────────────────
  return (
    <div class="flex h-full">
      {/* ── List pane ── */}
      <div class="flex w-56 shrink-0 flex-col border-r theme-border">
        <div class="p-2.5 pb-1">
          <ActionButton
            label="+ New Role"
            variant="ghost"
            class="w-full"
            onClick={openCreate}
          />
        </div>
        {/* Scope Filter Tabs */}
        <div class="flex items-center gap-1 px-2.5 py-1.5 border-b theme-border text-[10px]">
          <button
            type="button"
            onClick={() => setRoleFilter("all")}
            class={`px-2 py-0.5 rounded transition-colors ${roleFilter() === "all" ? "bg-[var(--ui-surface-muted)] theme-text font-semibold" : "theme-muted hover:theme-text"}`}
          >
            All ({userRoles().length})
          </button>
          <Show when={props.currentProject?.()}>
            {(proj) => (
              <button
                type="button"
                onClick={() => setRoleFilter("project")}
                class={`px-2 py-0.5 rounded transition-colors ${roleFilter() === "project" ? "bg-blue-500/20 text-blue-400 font-semibold" : "theme-muted hover:theme-text"}`}
              >
                Project ({projectRoles().length})
              </button>
            )}
          </Show>
          <button
            type="button"
            onClick={() => setRoleFilter("global")}
            class={`px-2 py-0.5 rounded transition-colors ${roleFilter() === "global" ? "bg-[var(--ui-surface-muted)] theme-text font-semibold" : "theme-muted hover:theme-text"}`}
          >
            Global ({globalRoles().length})
          </button>
        </div>
        <div class="flex-1 overflow-y-auto space-y-0.5 py-1">
          <Show when={filteredRoles().length === 0}>
            <EmptyState icon="◎" title="No roles" sub={roleFilter() === "project" ? "No roles configured for this project" : "Create your first role"} />
          </Show>
          <For each={filteredRoles()}>
            {(role) => {
              const color = () => RUNTIME_COLOR[role.runtimeKind] ?? "theme-muted";
              const isSelected = () => selectedId() === role.id && !creating();
              const isNative = () =>
                role.runtimeKind.includes("native") ||
                role.runtimeKind === "antigravity-cli" ||
                role.runtimeKind === "codex-cli" ||
                role.runtimeKind === "pi-cli";
              const isOverridden = () =>
                !role.projectId &&
                projectRoles().some((pr) => pr.roleName.toLowerCase() === role.roleName.toLowerCase());

              return (
                <div
                  onClick={() => openEdit(role)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setRoleContextMenu({ x: e.clientX, y: e.clientY, role });
                  }}
                  class={`group flex w-full flex-col gap-0.5 rounded-lg mx-1.5 px-2.5 py-2 text-left transition-colors duration-100 cursor-default ${isSelected() ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"}`}
                >
                  <div class="flex items-center justify-between min-w-0 gap-1">
                    <div class="flex items-center gap-1.5 min-w-0 truncate">
                      <span class={`truncate font-mono text-[10px] font-semibold ${isSelected() ? "theme-text" : "theme-text"}`}>{role.roleName}</span>
                      <Show when={role.projectId} fallback={
                        <span class="font-mono text-[8px] px-1.5 py-0.5 rounded bg-[var(--ui-surface-muted)] text-[var(--ui-muted)] border border-[var(--ui-border)]">global</span>
                      }>
                        <Show when={isProjectOverride(role)} fallback={
                          <span class="font-mono text-[8px] px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-400 border border-blue-500/25">project</span>
                        }>
                          <span class="font-mono text-[8px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/25">override</span>
                        </Show>
                      </Show>
                      <Show when={isOverridden()}>
                        <span
                          title="This global role is overridden by a project role of the same name"
                          class="font-mono text-[7.5px] px-1 py-0.2 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20"
                        >
                          overridden
                        </span>
                      </Show>
                    </div>
                    <Show when={deletingId() === role.id} fallback={
                      <button
                        onClick={(e) => { e.stopPropagation(); setDeletingId(role.id); }}
                        class="shrink-0 opacity-0 group-hover:opacity-100 theme-muted hover:text-rose-400 transition-all"
                        title={isProjectOverride(role) ? "Revert to Global Role" : "Delete Role"}
                      >
                        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                      </button>
                    }>
                      <div class="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                        <button onClick={() => void handleDelete(role)} class="font-mono text-[9px] text-rose-400 hover:text-rose-300">
                          {isProjectOverride(role) ? "revert" : "del"}
                        </button>
                        <button onClick={() => setDeletingId(null)} class="font-mono text-[9px] theme-muted hover:text-primary">✕</button>
                      </div>
                    </Show>
                  </div>
                  <div class="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
                    <span class={`font-mono text-[8.5px] px-1 py-0.2 rounded border ${isNative() ? "bg-amber-500/10 text-amber-400 border-amber-500/25" : "bg-blue-500/10 text-blue-400 border-blue-500/25"}`}>
                      {isNative() ? "⚡ Native" : "🔌 ACP"}
                    </span>
                    <span class={`font-mono text-[9px] ${color()}`}>{role.runtimeKind}</span>
                    <Show when={roleModel(role)}><span class="min-w-0 max-w-full truncate font-mono text-[9px] text-blue-400">{roleModel(role)}</span></Show>
                    <Show when={roleMode(role)}><span class="min-w-0 max-w-full truncate font-mono text-[9px] text-violet-300">mode:{roleMode(role)}</span></Show>
                    <Show when={roleEffort(role)}><span class="min-w-0 max-w-full truncate font-mono text-[9px] text-amber-300">effort:{roleEffort(role)}</span></Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </div>

      {/* ── Detail pane ── */}
      <div class="flex-1 overflow-y-auto p-5">

        {/* Create form */}
        <Show when={creating()}>
          <div class="space-y-4">
            <div class="flex items-center justify-between">
              <h3 class="font-mono text-xs font-bold theme-text uppercase tracking-widest">New Role</h3>
              <Show when={props.currentProject?.()}>
                {(proj) => (
                  <span class="text-[11px] text-blue-400 font-mono">
                    Project: {proj().name}
                  </span>
                )}
              </Show>
            </div>
            <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
              <FieldRow label="Scope">
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    disabled={!props.currentProject?.()}
                    onClick={() => setCScope("project")}
                    class={`px-2.5 py-1 text-xs rounded border transition-colors ${
                      cScope() === "project"
                        ? "bg-blue-500/20 text-blue-400 border-blue-500/40 font-semibold"
                        : "bg-[var(--ui-surface)] text-[var(--ui-muted)] border-[var(--ui-border)] hover:text-[var(--ui-text)]"
                    } ${!props.currentProject?.() ? "opacity-40 cursor-not-allowed" : ""}`}
                  >
                    Project: {props.currentProject?.() ? props.currentProject()!.name : "(No project)"}
                  </button>
                  <button
                    type="button"
                    onClick={() => setCScope("global")}
                    class={`px-2.5 py-1 text-xs rounded border transition-colors ${
                      cScope() === "global"
                        ? "bg-[var(--ui-accent-muted)] text-[var(--ui-accent)] border-[var(--ui-accent)] font-semibold"
                        : "bg-[var(--ui-surface)] text-[var(--ui-muted)] border-[var(--ui-border)] hover:text-[var(--ui-text)]"
                    }`}
                  >
                    Global (All Projects)
                  </button>
                </div>
              </FieldRow>
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
                <div class="w-full">
                  <InlineSelect value={cRuntime()} options={runtimeOptions()} onChange={(v) => {
                  const reqSeq = ++createConfigReqSeq;
                  setCRuntime(v); setCModel(""); setCMode(""); setCConfigSel({}); setCConfigOpts([]); setCModes([]);
                  setCConfigLoading(true);
                  void props.fetchRoleConfig(`runtime:${v}`, undefined, true)
                    .then(({ options, modes }) => {
                      if (reqSeq !== createConfigReqSeq || !creating() || cRuntime() !== v) return;
                      setCConfigOpts(options);
                      setCModes(modes);
                    })
                    .catch(() => {
                      if (reqSeq !== createConfigReqSeq) return;
                      setCConfigOpts([]);
                      setCModes([]);
                    })
                    .finally(() => {
                      if (reqSeq === createConfigReqSeq) setCConfigLoading(false);
                    });
                }} />
                  <CapabilityChips assistant={selectedAssistant()} />
                  {/* Runtime Mode Explanatory Box */}
                  <div class="mt-2 rounded p-2.5 border text-xs flex flex-col gap-1 border-[var(--ui-border)] bg-[var(--ui-surface)]">
                    <div class="flex items-center gap-2">
                      <span class={`font-mono text-[9px] font-bold px-1.5 py-0.5 rounded border ${
                        selectedAssistant()?.family === "native"
                          ? "bg-amber-500/15 text-amber-400 border-amber-500/30"
                          : "bg-blue-500/15 text-blue-400 border-blue-500/30"
                      }`}>
                        {selectedAssistant()?.family === "native" ? "⚡ Native Mode" : "🔌 ACP Mode"}
                      </span>
                      <span class="font-semibold theme-text text-[11px]">
                        {selectedAssistant()?.family === "native" ? "Native Direct CLI Execution" : "Agent Client Protocol Bridge"}
                      </span>
                    </div>
                    <p class="theme-muted text-[10px] leading-relaxed">
                      {selectedAssistant()?.family === "native"
                        ? "Direct subprocess invocation using native streaming JSON (`--output-format=stream-json`). Lowest latency, native tools & permissions, no protocol translation."
                        : "Connects via Agent Client Protocol (JSON-RPC 2.0). Standardized MCP server injection, dynamic mode toggles, and multi-agent coordination."}
                    </p>
                  </div>
                </div>
              </FieldRow>
              <FieldRow label="Prompt (Optional)">
                <div class="w-full flex flex-col gap-1">
                  <TextInput
                    value={cPrompt()}
                    onInput={setCPrompt}
                    placeholder="Optional override. Leave empty to use agent's native system prompt and project rules (CLAUDE.md, AGENTS.md)..."
                    multiline
                    rows={3}
                  />
                  <span class="text-[10px] theme-muted">
                    ⚡ Native Mode: Leave empty to preserve the agent's built-in reasoning and repository instructions.
                  </span>
                </div>
              </FieldRow>
              <FieldRow label="Model">
                <Show when={!cConfigLoading()} fallback={<span class="font-mono text-[10px] theme-muted">loading…</span>}>
                  <InlineSelect
                    value={cModel()}
                    options={resolvedModelOptions(cRuntime(), resolvedCreateConfigOpts().find(isModelOption))}
                    onChange={setCModel}
                  />
                </Show>
              </FieldRow>
              <Show when={resolvedModes(resolvedCreateConfigOpts(), cModes())}>
                {(mr) => {
                  const opts = () => mr().kind === "option"
                    ? configOptionSelectOptions(mr().opt!)
                    : [{ value: "", label: "default" }, ...mr().modes!.map((m) => ({ value: m, label: m }))];
                  return (
                    <FieldRow label="Mode">
                      <InlineSelect value={cMode()} options={opts()} onChange={setCMode} />
                    </FieldRow>
                  );
                }}
              </Show>
              <ParameterFields
                declared={readRuntimeOptions(resolvedCreateConfigOpts())}
                modelId={cModel()}
                value={(id) => cConfigSel()[id] ?? ""}
                onChange={(id, val) => setCConfigSel((prev) => ({ ...prev, [id]: val }))}
              />
              <Show when={cGlobalMcp().length > 0}>
                <FieldRow label="MCP">
                  <Show when={supportsCapability(cRuntime(), "mcpServers")} fallback={<span class="text-[10px] theme-muted">MCP is not supported by this runtime profile.</span>}>
                    <div class="space-y-1">
                      <For each={cGlobalMcp()}>{(entry) => (
                        <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                          <input
                            type="checkbox"
                            checked={entry.enabled}
                            onChange={(e) => setCGlobalMcp((prev) => prev.map((m) => m.mcpServerName === entry.mcpServerName ? { ...m, enabled: e.currentTarget.checked } : m))}
                            class="accent-indigo-400 h-3 w-3 shrink-0"
                          />
                          <span class="flex-1 truncate font-mono text-[10px] theme-text">{entry.mcpServerName}</span>
                          <Show when={entry.isBuiltin}>
                            <span class="text-[9px] theme-muted italic">builtin</span>
                          </Show>
                        </div>
                      )}</For>
                    </div>
                  </Show>
                </FieldRow>
              </Show>
              <Show when={cRoleRules().length > 0}>
                <FieldRow label="Rules">
                  <div class="space-y-1">
                    <For each={cRoleRules()}>{(rule) => (
                      <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                        <input
                          type="checkbox"
                          checked={rule.enabled}
                          onChange={(e) => setCRoleRules((prev) => prev.map((r) => r.ruleId === rule.ruleId ? { ...r, enabled: e.currentTarget.checked } : r))}
                          class="accent-violet-400 h-3 w-3 shrink-0"
                        />
                        <span class="flex-1 truncate font-mono text-[10px] theme-text">{rule.name}</span>
                        <Show when={rule.description}>
                          <span class="text-[9px] theme-muted italic truncate max-w-[120px]">{rule.description}</span>
                        </Show>
                      </div>
                    )}</For>
                  </div>
                </FieldRow>
              </Show>
              <Show when={cRoleSkills().length > 0}>
                <FieldRow label="Skills">
                  <div class="space-y-1">
                    <For each={cRoleSkills()}>{(skill) => (
                      <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                        <input
                          type="checkbox"
                          checked={skill.enabled}
                          onChange={(e) => setCRoleSkills((prev) => prev.map((s) => s.skillId === skill.skillId ? { ...s, enabled: e.currentTarget.checked } : s))}
                          class="accent-teal-400 h-3 w-3 shrink-0"
                        />
                        <span class="flex-1 truncate font-mono text-[10px] theme-text">{skill.name}</span>
                        <Show when={skill.description}>
                          <span class="text-[9px] theme-muted italic truncate max-w-[120px]">{skill.description}</span>
                        </Show>
                      </div>
                    )}</For>
                  </div>
                </FieldRow>
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
            const modelOpt = createMemo(() => resolvedEditConfigOpts().find(isModelOption));
            const modeResolved = createMemo(() => resolvedModes(resolvedEditConfigOpts(), eModes()));
            return (
              <div class="space-y-4">
                <div class="flex items-start justify-between gap-4">
                  <div>
                    <div class="flex items-center gap-2 flex-wrap">
                      <h2 class="font-mono text-sm font-bold theme-text">{role().roleName}</h2>
                      <span class={`font-mono text-[9px] font-semibold px-1.5 py-0.5 rounded border ${
                        (role().runtimeKind.includes("native") || role().runtimeKind === "antigravity-cli" || role().runtimeKind === "codex-cli" || role().runtimeKind === "pi-cli")
                          ? "bg-amber-500/15 text-amber-400 border-amber-500/30"
                          : "bg-blue-500/15 text-blue-400 border-blue-500/30"
                      }`}>
                        {(role().runtimeKind.includes("native") || role().runtimeKind === "antigravity-cli" || role().runtimeKind === "codex-cli" || role().runtimeKind === "pi-cli")
                          ? "⚡ Native Mode"
                          : "🔌 ACP Mode"}
                      </span>
                      <Show when={role().projectId} fallback={
                        <span class="font-mono text-[9px] font-semibold px-2 py-0.5 rounded bg-[var(--ui-surface-muted)] text-[var(--ui-muted)] border border-[var(--ui-border)]">
                          Global Role
                        </span>
                      }>
                        <span class="font-mono text-[9px] font-semibold px-2 py-0.5 rounded bg-blue-500/20 text-blue-400 border border-blue-500/40">
                          Project Role · {props.currentProject?.()?.name ?? "Project"}
                        </span>
                      </Show>
                    </div>
                    <div class="flex items-center gap-2 mt-1">
                      <span class={`font-mono text-[10px] ${RUNTIME_COLOR[role().runtimeKind] ?? "theme-muted"}`}>{role().runtimeKind}</span>
                      <Show when={role().runtimeLaunchMethod}>
                        {(method) => <span class="font-mono text-[9px] theme-muted">{method()}</span>}
                      </Show>
                    </div>
                  </div>
                  <div class="flex items-center gap-2 shrink-0">
                    <Show when={!role().projectId && props.currentProject?.()}>
                      {(proj) => (
                        <button
                          type="button"
                          onClick={() => void copyRoleToProject(role(), proj())}
                          disabled={saving()}
                          class="px-2.5 py-1 text-xs rounded border border-blue-500/30 bg-blue-500/15 text-blue-300 hover:bg-blue-500/25 transition-colors font-medium"
                        >
                          Copy to {proj().name}
                        </button>
                      )}
                    </Show>
                    <Show when={deletingId() === role().id} fallback={
                      <ActionButton
                        label={isProjectOverride(role()) ? "Revert to Global" : "Delete"}
                        variant={isProjectOverride(role()) ? "secondary" : "danger"}
                        onClick={() => setDeletingId(role().id)}
                      />
                    }>
                      <div class="flex items-center gap-2">
                        <ActionButton
                          label={
                            deletingRole() === role().id
                              ? (isProjectOverride(role()) ? "Reverting..." : "Deleting...")
                              : (isProjectOverride(role()) ? "Confirm Revert" : "Confirm delete")
                          }
                          variant={isProjectOverride(role()) ? "secondary" : "danger"}
                          disabled={deletingRole() === role().id}
                          onClick={() => void handleDelete(role())}
                        />
                        <ActionButton label="Cancel" variant="ghost" onClick={() => setDeletingId(null)} />
                      </div>
                    </Show>
                  </div>
                </div>

                {/* Helpful Role Scope Guidance Banner */}
                <Show when={!role().projectId && props.currentProject?.()}>
                  {(proj) => (
                    <div class="flex items-center justify-between gap-3 p-3 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-200">
                      <div class="flex items-center gap-2">
                        <Globe size={15} class="text-amber-400 shrink-0" />
                        <span>
                          <strong>Global Role:</strong> Shared across all projects. To customize settings specifically for <strong>{proj().name}</strong>, click "Copy to {proj().name}".
                        </span>
                      </div>
                      <button
                        type="button"
                        onClick={() => void copyRoleToProject(role(), proj())}
                        class="shrink-0 px-2.5 py-1 rounded bg-blue-500/20 hover:bg-blue-500/30 text-blue-300 border border-blue-500/40 text-[11px] font-semibold transition-colors"
                      >
                        Copy to {proj().name}
                      </button>
                    </div>
                  )}
                </Show>

                <Show when={role().projectId}>
                  <div class="flex items-center gap-2 p-3 rounded-lg border border-blue-500/30 bg-blue-500/10 text-xs text-blue-200">
                    <FolderGit2 size={15} class="text-blue-400 shrink-0" />
                    <span>
                      {isProjectOverride(role()) ? (
                        <>
                          <strong>Project Override:</strong> This role overrides the global <strong>{role().roleName}</strong> role for <strong>{props.currentProject?.()?.name ?? "this project"}</strong>. Click "Revert to Global" to return to the global settings.
                        </>
                      ) : (
                        <>
                          <strong>Project Role:</strong> Configured specifically for <strong>{props.currentProject?.()?.name ?? "this project"}</strong>. Sessions in this project use this role instead of global fallbacks.
                        </>
                      )}
                    </span>
                  </div>
                </Show>
                <Show when={deleteError()}>
                  <div class="management-error-box font-mono">
                    {deleteError()}
                  </div>
                </Show>

                <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
                  <FieldRow label="Scope">
                    <div class="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!props.currentProject?.()}
                        onClick={() => setEScope("project")}
                        class={`px-2.5 py-1 text-xs rounded border transition-colors ${
                          eScope() === "project"
                            ? "bg-blue-500/20 text-blue-400 border-blue-500/40 font-semibold"
                            : "bg-[var(--ui-surface)] text-[var(--ui-muted)] border-[var(--ui-border)] hover:text-[var(--ui-text)]"
                        } ${!props.currentProject?.() ? "opacity-40 cursor-not-allowed" : ""}`}
                      >
                        Project {props.currentProject?.() ? `(${props.currentProject()!.name})` : "(No project selected)"}
                      </button>
                      <button
                        type="button"
                        onClick={() => setEScope("global")}
                        class={`px-2.5 py-1 text-xs rounded border transition-colors ${
                          eScope() === "global"
                            ? "bg-[var(--ui-accent-muted)] text-[var(--ui-accent)] border-[var(--ui-accent)] font-semibold"
                            : "bg-[var(--ui-surface)] text-[var(--ui-muted)] border-[var(--ui-border)] hover:text-[var(--ui-text)]"
                        }`}
                      >
                        Global (All Projects)
                      </button>
                    </div>
                  </FieldRow>
                  <FieldRow label="Prompt (Optional)">
                    <div class="w-full flex flex-col gap-1">
                      <TextInput
                        value={ePrompt()}
                        onInput={setEPrompt}
                        placeholder="Optional override. Leave empty to use agent's native system prompt and project rules (CLAUDE.md, AGENTS.md)..."
                        multiline
                        rows={3}
                      />
                      <span class="text-[10px] theme-muted">
                        ⚡ Native Mode: Leave empty to preserve the agent's built-in reasoning and repository instructions.
                      </span>
                    </div>
                  </FieldRow>
                  <FieldRow label="Model">
                    <div class="flex items-center gap-2">
                      <div class="min-w-0 flex-1">
                        <Show when={!eConfigLoading()} fallback={<span class="font-mono text-[10px] theme-muted">loading…</span>}>
                          <InlineSelect
                            value={eModel()}
                            options={resolvedModelOptions(role().runtimeKind, modelOpt())}
                            onChange={setEModel}
                          />
                        </Show>
                      </div>
                      <button
                        type="button"
                        title="Refresh model list from the runtime"
                        onClick={refreshEditConfig}
                        disabled={eConfigLoading()}
                        class="shrink-0 rounded border theme-border px-2 py-1 text-[9px] theme-muted hover:text-primary disabled:opacity-40"
                      >
                        {eConfigLoading() ? "…" : "↻"}
                      </button>
                    </div>
                  </FieldRow>
                  <Show when={modeResolved()}>
                    {(mr) => {
                      const opts = () => mr().kind === "option"
                        ? configOptionSelectOptions(mr().opt!)
                        : [{ value: "", label: "default" }, ...mr().modes!.map((m) => ({ value: m, label: m }))];
                      return (
                        <FieldRow label="Mode">
                          <InlineSelect value={eMode()} options={opts()} onChange={setEMode} />
                        </FieldRow>
                      );
                    }}
                  </Show>
                  <ParameterFields
                    declared={readRuntimeOptions(resolvedEditConfigOpts())}
                    modelId={eModel()}
                    value={(id) => editCfgMap()[id] ?? ""}
                    onChange={updateEditCfg}
                  />
                  <Show when={eGlobalMcp().length > 0}>
                    <FieldRow label="MCP">
                      <Show when={supportsCapability(role().runtimeKind, "mcpServers")} fallback={<span class="text-[10px] theme-muted">MCP is not supported by this runtime profile.</span>}>
                        <div class="space-y-1">
                          <For each={eGlobalMcp()}>{(entry) => (
                            <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                              <input
                                type="checkbox"
                                checked={entry.enabled}
                                onChange={(e) => void handleToggleGlobalMcp(entry, e.currentTarget.checked)}
                                class="accent-indigo-400 h-3 w-3 shrink-0"
                              />
                              <span class="flex-1 truncate font-mono text-[10px] theme-text">{entry.mcpServerName}</span>
                              <Show when={entry.isBuiltin}>
                                <span class="text-[9px] theme-muted italic">builtin</span>
                              </Show>
                            </div>
                          )}</For>
                          <Show when={mcpResetting()}>
                            <div class="text-[9.5px] text-amber-300 font-mono pt-0.5">MCP changed — reconnecting live sessions…</div>
                          </Show>
                        </div>
                      </Show>
                    </FieldRow>
                  </Show>
                  <Show when={eRoleRules().length > 0}>
                    <FieldRow label="Rules">
                      <div class="space-y-1">
                        <For each={eRoleRules()}>{(rule) => (
                          <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                            <input
                              type="checkbox"
                              checked={rule.enabled}
                              onChange={(e) => void handleToggleRule(rule, e.currentTarget.checked)}
                              class="accent-violet-400 h-3 w-3 shrink-0"
                            />
                            <span class="flex-1 truncate font-mono text-[10px] theme-text">{rule.name}</span>
                            <Show when={rule.description}>
                              <span class="text-[9px] theme-muted italic truncate max-w-[120px]">{rule.description}</span>
                            </Show>
                          </div>
                        )}</For>
                      </div>
                    </FieldRow>
                  </Show>
                  <Show when={eRoleSkills().length > 0}>
                    <FieldRow label="Skills">
                      <div class="space-y-1">
                        <For each={eRoleSkills()}>{(skill) => (
                          <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1">
                            <input
                              type="checkbox"
                              checked={skill.enabled}
                              onChange={(e) => void handleToggleSkill(skill, e.currentTarget.checked)}
                              class="accent-teal-400 h-3 w-3 shrink-0"
                            />
                            <span class="flex-1 truncate font-mono text-[10px] theme-text">{skill.name}</span>
                            <Show when={skill.description}>
                              <span class="text-[9px] theme-muted italic truncate max-w-[120px]">{skill.description}</span>
                            </Show>
                          </div>
                        )}</For>
                      </div>
                    </FieldRow>
                  </Show>
                </div>

                <div class="flex gap-2">
                  <ActionButton label={saving() ? "Saving…" : "Save"} variant="primary" disabled={saving()} onClick={() => void handleSaveEdit()} />
                  <ActionButton label="Back" variant="ghost" onClick={() => setSelectedId(null)} />
                </div>
              </div>
            );
          }}
        </Show>

        {/* Empty state */}
        <Show when={!creating() && !editingRole()}>
          <EmptyState icon="◎" title="Select a role" sub="Or create a new one" />
        </Show>
      </div>

      {/* ── Role Context Menu ── */}
      <Show when={roleContextMenu()}>
        {(menu) => (
          <ContextMenuSurface
            x={menu().x}
            y={menu().y}
            width={200}
            onClick={(e) => e.stopPropagation()}
          >
            <ContextMenuItem
              icon={<Edit3 size={13} />}
              onSelect={() => {
                closeRoleContextMenu();
                openEdit(menu().role);
              }}
            >
              Edit Role
            </ContextMenuItem>
            <Show
              when={menu().role.projectId}
              fallback={
                <Show when={props.currentProject?.()}>
                  {(proj) => (
                    <>
                      <ContextMenuItem
                        icon={<Copy size={13} class="text-blue-400" />}
                        onSelect={() => {
                          const r = menu().role;
                          closeRoleContextMenu();
                          void copyRoleToProject(r, proj());
                        }}
                      >
                        Copy to {proj().name}
                      </ContextMenuItem>
                      <ContextMenuItem
                        icon={<FolderGit2 size={13} class="text-blue-400" />}
                        onSelect={async () => {
                          const r = menu().role;
                          closeRoleContextMenu();
                          try {
                            await roleApi.reassignProject(r.roleName, proj().id, r.id);
                            await props.refreshRoles(props.currentProject?.()?.id);
                            props.pushMessage("event", `Role '${r.roleName}' assigned to project '${proj().name}'`);
                          } catch (e) {
                            const err = parseError(e);
                            props.pushMessage("event", `Failed to assign role: ${err.message}`);
                          }
                        }}
                      >
                        Move to {proj().name}
                      </ContextMenuItem>
                    </>
                  )}
                </Show>
              }
            >
              <ContextMenuItem
                icon={<Globe size={13} class="text-amber-400" />}
                onSelect={async () => {
                  const r = menu().role;
                  closeRoleContextMenu();
                  try {
                    await roleApi.reassignProject(r.roleName, null, r.id);
                    await props.refreshRoles(props.currentProject?.()?.id);
                    props.pushMessage("event", `Role '${r.roleName}' moved to Global scope`);
                  } catch (e) {
                    const err = parseError(e);
                    props.pushMessage("event", `Failed to make role global: ${err.message}`);
                  }
                }}
              >
                Move to Global Scope
              </ContextMenuItem>
            </Show>
            <ContextMenuItem
              icon={<Files size={13} />}
              onSelect={() => {
                const r = menu().role;
                closeRoleContextMenu();
                void duplicateRole(r);
              }}
            >
              Duplicate Role
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              icon={<Trash2 size={13} class="text-red-400" />}
              class="text-red-400 hover:text-red-300"
              onSelect={() => {
                const r = menu().role;
                closeRoleContextMenu();
                void handleDelete(r);
              }}
            >
              Delete Role
            </ContextMenuItem>
          </ContextMenuSurface>
        )}
      </Show>
    </div>
  );
}
