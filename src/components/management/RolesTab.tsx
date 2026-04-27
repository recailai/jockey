import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, Role, RoleUpsertInput, AcpConfigOption } from "../types";
import { RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "../types";
import { EmptyState, FieldRow, TextInput, InlineSelect, ActionButton } from "./primitives";
import { roleApi, assistantApi, globalMcpApi, ruleApi, skillApi, parseError } from "../../lib/tauriApi";
import type { RoleMcpEntry, RoleRule, RoleSkill } from "../../lib/tauriApi";
import { codexReasoningEffortOption, isEffortOption, isModeOption, isModelOption, optionCurrentValue, optionId, optionName } from "../../lib/configOptions";

export function RolesTab(props: {
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  refreshRoles: () => Promise<void>;
  fetchRoleConfig: (runtimeKey: string, roleName?: string) => Promise<{ options: AcpConfigOption[]; modes: string[] }>;
  pushMessage: (role: string, text: string) => void;
  initialRoleName?: string;
}) {
  const UNION_ROLE = "Jockey";
  const userRoles = createMemo(() => props.roles().filter((r) => r.roleName !== UNION_ROLE));

  // "creating" = create form open; selectedId = which role is being edited
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [deletingId, setDeletingId] = createSignal<string | null>(null);
  const [deleteError, setDeleteError] = createSignal<string | null>(null);
  const [deletingRole, setDeletingRole] = createSignal<string | null>(null);

  // ── Create form state ───────────────────────────────────────────────────────
  const [cName, setCName] = createSignal("Developer");
  const [cRuntime, setCRuntime] = createSignal("claude-code");
  const [cPrompt, setCPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
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
    const defaultRuntime = "claude-code";
    const reqSeq = ++createConfigReqSeq;
    setCName("Developer");
    setCRuntime(defaultRuntime);
    setCPrompt("You are a senior developer. Implement the solution step by step.");
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
    setCModes([]);
    setCConfigLoading(true);
    void props.fetchRoleConfig(`runtime:${defaultRuntime}`)
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
    setEPrompt(role.systemPrompt ?? "");
    setEModel(role.model ?? "");
    setEMode(role.mode ?? "");
    setECfgJson(role.configOptionsJson || "{}");
    setEConfigOpts([]);
    setEModes([]);
    setERoleRules([]);
    setERoleSkills([]);
    void props.fetchRoleConfig(role.runtimeKind, role.roleName)
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
      });
    void globalMcpApi.listRoleMcp(role.roleName).then(setEGlobalMcp).catch(() => {});
    void ruleApi.listAllRulesForRole(role.roleName).then(setERoleRules).catch(() => {});
    void skillApi.listAllSkillsForRole(role.roleName).then(setERoleSkills).catch(() => {});
  };

  // Auto-open edit when panel is opened from sidebar with a pre-selected role name.
  // Guard: only run once (when selectedId is still null and creating is false).
  createEffect(() => {
    if (!props.initialRoleName || creating() || selectedId()) return;
    const role = userRoles().find((r) => r.roleName === props.initialRoleName);
    if (role) openEdit(role);
  });

  // ── Helpers ─────────────────────────────────────────────────────────────────
  const uniqueRoleName = (desired: string): string => {
    const existing = new Set(userRoles().map((r) => r.roleName.toLowerCase()));
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

  const roleEffort = (role: Role) => {
    const cfg = parseRoleConfigMap(role.configOptionsJson);
    const defs = roleConfigDefs(role);
    const opt = defs.find(isEffortOption);
    const exactId = opt ? optionId(opt) : "";
    if (role.runtimeKind === "codex-cli") {
      if (exactId && cfg[exactId]) return cfg[exactId];
      const override = cfg.reasoning_effort ?? cfg.thinking_effort ?? cfg.effort ?? cfg.thought_level;
      if (override) return override;
      return opt ? optionCurrentValue(opt) || null : null;
    }
    if (cfg.effort) {
      return cfg.effort;
    }
    return exactId === "effort" && opt ? optionCurrentValue(opt) || null : null;
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

  const configOptionSelectOptions = (opt: AcpConfigOption, defaultLabel = "default") =>
    [{ value: "", label: `${defaultLabel}: ${optionCurrentValue(opt) || "runtime"}` }, ...flattenConfigValues(opt.options).map((v) => ({ value: v.value, label: v.description ? `${v.name} — ${v.description}` : v.name }))];

  // ── Create submit ───────────────────────────────────────────────────────────
  const handleCreate = async () => {
    const name = uniqueRoleName(cName().trim());
    if (!name || saving()) return;
    if (/\s/.test(name)) { props.pushMessage("event", "Role name cannot contain spaces."); return; }
    if (!/^[A-Za-z0-9_-]+$/.test(name)) { props.pushMessage("event", "Role name only allows letters, numbers, - and _."); return; }
    setSaving(true);
    const configMap: Record<string, string> = {};
    for (const [k, v] of Object.entries(cConfigSel())) { if (v) configMap[k] = v; }
    try {
      const saved = await roleApi.upsert({
        roleName: name, runtimeKind: cRuntime(),
        systemPrompt: cPrompt().trim() || "You are a helpful AI assistant.",
        model: cModel().trim() || null, mode: cMode().trim() || null,
        mcpServersJson: "[]", configOptionsJson: JSON.stringify(configMap),
        configOptionDefsJson: JSON.stringify(resolvedCreateConfigOpts()),
        autoApprove: true,
      } satisfies RoleUpsertInput);
      await Promise.all(cGlobalMcp().map((entry) =>
        globalMcpApi.setRoleMcpEnabled(name, entry.mcpServerName, entry.enabled).catch(() => {}),
      ));
      await ruleApi.setRoleRules(
        name,
        cRoleRules().map((r) => [r.ruleId, r.enabled, r.ord] as [string, boolean, number]),
      ).catch(() => {});
      await skillApi.setRoleSkills(
        name,
        cRoleSkills().map((s) => [s.skillId, s.enabled, s.ord] as [string, boolean, number]),
      ).catch(() => {});
      await props.refreshRoles();
      // 创建成功后直接进入编辑状态
      const created = props.roles().find((r) => r.roleName === saved.roleName);
      if (created) {
        openEdit(created);
      } else {
        setCreating(false);
      }
      setCConfigSel({}); setCConfigOpts([]); setCGlobalMcp([]); setCRoleRules([]); setCRoleSkills([]);
      props.pushMessage("event", `role created: ${saved.roleName} (${saved.runtimeKind})`);
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
    setSaving(true);
    try {
      await roleApi.upsert({
        roleName: role.roleName, runtimeKind: role.runtimeKind,
        systemPrompt: ePrompt().trim(), model: eModel().trim() || null,
        mode: newMode, mcpServersJson: "[]",
        configOptionsJson: JSON.stringify(parsedCfg),
        configOptionDefsJson: JSON.stringify(resolvedEditConfigOpts()),
        autoApprove: true,
      } satisfies RoleUpsertInput);
      await props.refreshRoles();
      if (modeChanged && newMode) {
        const synced = await assistantApi.syncRoleMode(role.roleName, newMode).catch(() => []);
        for (const sessionId of synced) {
          props.updateSession(sessionId, { currentMode: newMode });
        }
      }
      props.pushMessage("event", `role saved: ${role.roleName}`);
    } catch (e) { const err = parseError(e); props.pushMessage("event", `Failed to save: ${err.message}`); }
    finally { setSaving(false); }
  };

  // ── Delete ──────────────────────────────────────────────────────────────────
  const handleDelete = async (roleName: string) => {
    if (deletingRole()) return;
    setDeleteError(null);
    setDeletingRole(roleName);
    try {
      await roleApi.remove(roleName);
      if (editingRole()?.roleName === roleName) setSelectedId(null);
      setDeletingId(null);
      await props.refreshRoles();
      props.pushMessage("event", `role deleted: ${roleName}`);
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
    if (!role) return;
    await globalMcpApi.setRoleMcpEnabled(role.roleName, entry.mcpServerName, enabled).catch(() => {});
    setEGlobalMcp((prev) => prev.map((e) => e.mcpServerName === entry.mcpServerName ? { ...e, enabled } : e));
    setMcpResetting(true);
    await globalMcpApi.resetRoleMcpSessions(role.roleName).catch(() => {});
    setMcpResetting(false);
  };

  const handleToggleRule = async (rule: RoleRule, enabled: boolean) => {
    const role = editingRole();
    if (!role) return;
    const updated = eRoleRules().map((r) => r.ruleId === rule.ruleId ? { ...r, enabled } : r);
    setERoleRules(updated);
    const payload: [string, boolean, number][] = updated.map((r) => [r.ruleId, r.enabled, r.ord]);
    await ruleApi.setRoleRules(role.roleName, payload).catch(() => {});
  };

  const handleToggleSkill = async (skill: RoleSkill, enabled: boolean) => {
    const role = editingRole();
    if (!role) return;
    const updated = eRoleSkills().map((s) => s.skillId === skill.skillId ? { ...s, enabled } : s);
    setERoleSkills(updated);
    const payload: [string, boolean, number][] = updated.map((s) => [s.skillId, s.enabled, s.ord]);
    await skillApi.setRoleSkills(role.roleName, payload).catch(() => {});
  };

  const runtimeOptions = RUNTIMES.map((r) => ({ value: r, label: r }));

  // Edit form config options (local, not tied to global activeSession)
  const editConfigOpts = createMemo(() => eConfigOpts());
  const createConfigOpts = createMemo(() => {
    const opts = cConfigOpts();
    if (cRuntime() !== "codex-cli" || opts.some(isEffortOption) || !cModel()) return opts;
    return [...opts, codexReasoningEffortOption()];
  });
  const resolvedCreateConfigOpts = createMemo(() => createConfigOpts());
  const editCfgMap = createMemo((): Record<string, string> => {
    try { return JSON.parse(eCfgJson() || "{}"); } catch { return {}; }
  });
  const resolvedEditConfigOpts = createMemo(() => {
    const opts = editConfigOpts();
    if (editingRole()?.runtimeKind !== "codex-cli" || opts.some(isEffortOption) || !eModel()) return opts;
    const saved = editCfgMap().reasoning_effort || "medium";
    return [...opts, codexReasoningEffortOption(saved)];
  });
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
        <div class="p-3">
          <ActionButton
            label="+ New Role"
            variant="ghost"
            class="w-full"
            onClick={openCreate}
          />
        </div>
        <div class="flex-1 overflow-y-auto space-y-0.5 py-1">
          <Show when={userRoles().length === 0}>
            <EmptyState icon="◎" title="No roles yet" sub="Create your first role" />
          </Show>
          <For each={userRoles()}>
            {(role) => {
              const color = () => RUNTIME_COLOR[role.runtimeKind] ?? "theme-muted";
              const isSelected = () => selectedId() === role.id && !creating();
              return (
                <div
                  onClick={() => openEdit(role)}
                  class={`group flex w-full flex-col gap-0.5 rounded-lg mx-1.5 px-2.5 py-2 text-left transition-colors duration-100 cursor-default ${isSelected() ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"}`}
                >
                  <div class="flex items-center justify-between min-w-0 gap-1">
                    <span class={`truncate font-mono text-[10px] font-semibold ${isSelected() ? "theme-text" : "theme-text"}`}>{role.roleName}</span>
                    <Show when={deletingId() === role.roleName} fallback={
                      <button
                        onClick={(e) => { e.stopPropagation(); setDeletingId(role.roleName); }}
                        class="shrink-0 opacity-0 group-hover:opacity-100 theme-muted hover:text-rose-400 transition-all"
                      >
                        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                      </button>
                    }>
                      <div class="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                        <button onClick={() => void handleDelete(role.roleName)} class="font-mono text-[9px] text-rose-400 hover:text-rose-300">del</button>
                        <button onClick={() => setDeletingId(null)} class="font-mono text-[9px] theme-muted hover:text-primary">✕</button>
                      </div>
                    </Show>
                  </div>
                  <div class="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
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
            <h3 class="font-mono text-xs font-bold theme-text uppercase tracking-widest">New Role</h3>
            <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
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
                  const reqSeq = ++createConfigReqSeq;
                  setCRuntime(v); setCModel(""); setCMode(""); setCConfigSel({}); setCConfigOpts([]); setCModes([]);
                  setCConfigLoading(true);
                  void props.fetchRoleConfig(`runtime:${v}`)
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
              </FieldRow>
              <FieldRow label="Prompt">
                <TextInput value={cPrompt()} onInput={setCPrompt} placeholder="System prompt…" multiline rows={4} />
              </FieldRow>
              <FieldRow label="Model">
                <Show when={!cConfigLoading()} fallback={<span class="font-mono text-[10px] theme-muted">loading…</span>}>
                  <Show when={resolvedCreateConfigOpts().find(isModelOption)} fallback={
                    <TextInput value={cModel()} onInput={setCModel} placeholder="Optional model override" monospace />
                  }>
                    {(mo) => {
                      return (
                        <InlineSelect
                          value={cModel()}
                          options={configOptionSelectOptions(mo())}
                          onChange={setCModel}
                        />
                      );
                    }}
                  </Show>
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
              <Show when={resolvedCreateConfigOpts().find(isEffortOption)}>
                {(opt) => {
                  return (
                    <FieldRow label={optionName(opt()) || "Effort"}>
                      <InlineSelect
                        value={cConfigSel()[optionId(opt())] ?? ""}
                        options={configOptionSelectOptions(opt())}
                        onChange={(val) => setCConfigSel((s) => ({ ...s, [optionId(opt())]: val }))}
                      />
                    </FieldRow>
                  );
                }}
              </Show>
              <Show when={resolvedCreateConfigOpts().length > 0}>
                <For each={resolvedCreateConfigOpts().filter((o) => !isModelOption(o) && !isModeOption(o) && !isEffortOption(o))}>
                  {(opt) => {
                    return (
                      <FieldRow label={optionName(opt)}>
                        <InlineSelect
                          value={cConfigSel()[optionId(opt)] ?? ""}
                          options={configOptionSelectOptions(opt)}
                          onChange={(val) => setCConfigSel((s) => ({ ...s, [optionId(opt)]: val }))}
                        />
                      </FieldRow>
                    );
                  }}
                </For>
              </Show>
              <Show when={cGlobalMcp().length > 0}>
                <FieldRow label="MCP">
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
            const effortOpt = createMemo(() => resolvedEditConfigOpts().find(isEffortOption));
            const otherOpts = createMemo(() => resolvedEditConfigOpts().filter((o) => !isModelOption(o) && !isModeOption(o) && !isEffortOption(o)));
            return (
              <div class="space-y-4">
                <div class="flex items-start justify-between gap-4">
                  <div>
                    <h2 class="font-mono text-sm font-bold theme-text">{role().roleName}</h2>
                    <span class={`font-mono text-[10px] ${RUNTIME_COLOR[role().runtimeKind] ?? "theme-muted"}`}>{role().runtimeKind}</span>
                    <Show when={role().runtimeLaunchMethod}>
                      {(method) => <span class="font-mono text-[9px] theme-muted ml-2">{method()}</span>}
                    </Show>
                  </div>
                  <Show when={deletingId() === role().roleName} fallback={
                    <ActionButton label="Delete" variant="danger" onClick={() => setDeletingId(role().roleName)} />
                  }>
                    <div class="flex items-center gap-2">
                      <ActionButton
                        label={deletingRole() === role().roleName ? "Deleting..." : "Confirm delete"}
                        variant="danger"
                        disabled={deletingRole() === role().roleName}
                        onClick={() => void handleDelete(role().roleName)}
                      />
                      <ActionButton label="Cancel" variant="ghost" onClick={() => setDeletingId(null)} />
                    </div>
                  </Show>
                </div>
                <Show when={deleteError()}>
                  <div class="rounded-md border border-rose-500/30 bg-rose-500/10 px-3 py-2 font-mono text-[10px] text-rose-200">
                    {deleteError()}
                  </div>
                </Show>

                <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
                  <FieldRow label="Prompt">
                    <TextInput value={ePrompt()} onInput={setEPrompt} placeholder="System prompt" multiline rows={4} />
                  </FieldRow>
                  <FieldRow label="Model">
                    <Show when={modelOpt()} fallback={
                      <TextInput value={eModel()} onInput={setEModel} placeholder="Optional model" monospace />
                    }>
                      {(mo) => {
                        return (
                          <InlineSelect
                            value={eModel()}
                            options={configOptionSelectOptions(mo())}
                            onChange={setEModel}
                          />
                        );
                      }}
                    </Show>
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
                  <Show when={effortOpt()}>
                    {(opt) => {
                      return (
                        <FieldRow label={optionName(opt()) || "Effort"}>
                          <InlineSelect
                            value={editCfgMap()[optionId(opt())] ?? ""}
                            options={configOptionSelectOptions(opt())}
                            onChange={(val) => updateEditCfg(optionId(opt()), val)}
                          />
                        </FieldRow>
                      );
                    }}
                  </Show>
                  <For each={otherOpts()}>
                    {(opt) => {
                      return (
                        <FieldRow label={optionName(opt)}>
                          <InlineSelect
                            value={editCfgMap()[optionId(opt)] ?? ""}
                            options={configOptionSelectOptions(opt)}
                            onChange={(val) => updateEditCfg(optionId(opt), val)}
                          />
                        </FieldRow>
                      );
                    }}
                  </For>
                  <Show when={eGlobalMcp().length > 0}>
                    <FieldRow label="MCP">
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
    </div>
  );
}
