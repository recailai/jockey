import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, Role, RoleUpsertInput, AcpConfigOption } from "../types";
import { RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "../types";
import { EmptyState, FieldRow, TextInput, InlineSelect, ActionButton, Badge } from "./primitives";
import type { AcpMcpServer } from "./primitives";
import { mcpTransport, mcpDisplayUri, parseCommandArgs } from "./primitives";
import { roleApi, assistantApi, globalMcpApi, parseError } from "../../lib/tauriApi";
import type { RoleMcpEntry } from "../../lib/tauriApi";

export function RolesTab(props: {
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  refreshRoles: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
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

  // ── Create form state ───────────────────────────────────────────────────────
  const [cName, setCName] = createSignal("Developer");
  const [cRuntime, setCRuntime] = createSignal("claude-code");
  const [cPrompt, setCPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
  const [cModel, setCModel] = createSignal("");
  const [cMode, setCMode] = createSignal("");
  const [cAutoApprove, setCAutoApprove] = createSignal(true);
  const [cConfigOpts, setCConfigOpts] = createSignal<AcpConfigOption[]>([]);
  const [cConfigSel, setCConfigSel] = createSignal<Record<string, string>>({});

  // ── Edit form state ─────────────────────────────────────────────────────────
  const [ePrompt, setEPrompt] = createSignal("");
  const [eModel, setEModel] = createSignal("");
  const [eMode, setEMode] = createSignal("");
  const [eAutoApprove, setEAutoApprove] = createSignal(true);
  const [eMcpServers, setEMcpServers] = createSignal<AcpMcpServer[]>([]);
  const [eMcpAdding, setEMcpAdding] = createSignal(false);
  const [eMcpName, setEMcpName] = createSignal("");
  const [eMcpTransport, setEMcpTransport] = createSignal<"stdio" | "http" | "sse">("stdio");
  const [eMcpCommand, setEMcpCommand] = createSignal("");
  const [eMcpArgs, setEMcpArgs] = createSignal("");
  const [eMcpUrl, setEMcpUrl] = createSignal("");
  const [eCfgJson, setECfgJson] = createSignal("{}");
  const [eConfigOpts, setEConfigOpts] = createSignal<AcpConfigOption[]>([]);
  const [eGlobalMcp, setEGlobalMcp] = createSignal<RoleMcpEntry[]>([]);
  const [mcpResetting, setMcpResetting] = createSignal(false);

  const editingRole = createMemo(() =>
    selectedId() ? userRoles().find((r) => r.id === selectedId()) ?? null : null,
  );

  // ── Open create ─────────────────────────────────────────────────────────────
  const openCreate = () => {
    const defaultRuntime = "claude-code";
    setCName("Developer");
    setCRuntime(defaultRuntime);
    setCPrompt("You are a senior developer. Implement the solution step by step.");
    setCModel("");
    setCMode("");
    setCAutoApprove(true);
    setCConfigOpts([]);
    setCConfigSel({});
    setSelectedId(null);  // 退出 edit 状态
    setDeletingId(null);
    setCreating(true);    // 最后设，避免被 effect 覆盖
    void props.fetchConfigOptions(defaultRuntime).then(setCConfigOpts);
  };

  // ── Open edit ───────────────────────────────────────────────────────────────
  const openEdit = (role: Role) => {
    setCreating(false);
    setSelectedId(role.id);
    setEPrompt(role.systemPrompt ?? "");
    setEModel(role.model ?? "");
    setEMode(role.mode ?? "");
    setEAutoApprove(role.autoApprove);
    try { setEMcpServers(JSON.parse(role.mcpServersJson || "[]")); } catch { setEMcpServers([]); }
    setEMcpAdding(false);
    setECfgJson(role.configOptionsJson || "{}");
    setEConfigOpts([]);
    void props.fetchConfigOptions(role.runtimeKind, role.roleName).then(setEConfigOpts);
    void globalMcpApi.listRoleMcp(role.roleName).then(setEGlobalMcp).catch(() => {});
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

  const isModelOrMode = (o: AcpConfigOption) => {
    const id = o.id.toLowerCase();
    const cat = o.category?.toLowerCase();
    const name = o.name.toLowerCase();
    return id === "model" || id === "mode" || cat === "model" || cat === "mode" || name === "model" || name === "mode";
  };

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
        autoApprove: cAutoApprove(),
      } satisfies RoleUpsertInput);
      await props.refreshRoles();
      // 创建成功后直接进入编辑状态
      const created = props.roles().find((r) => r.roleName === saved.roleName);
      if (created) {
        openEdit(created);
      } else {
        setCreating(false);
      }
      setCConfigSel({}); setCConfigOpts([]);
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
        mode: newMode, mcpServersJson: JSON.stringify(eMcpServers()),
        configOptionsJson: JSON.stringify(parsedCfg), autoApprove: eAutoApprove(),
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
    try {
      await roleApi.remove(roleName);
      if (editingRole()?.roleName === roleName) setSelectedId(null);
      setDeletingId(null);
      await props.refreshRoles();
    } catch (e) { const err = parseError(e); props.pushMessage("event", `Failed to delete role: ${err.message}`); }
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

  const runtimeOptions = RUNTIMES.map((r) => ({ value: r, label: r }));

  // Edit form config options (local, not tied to global activeSession)
  const editConfigOpts = createMemo(() => eConfigOpts());
  const editCfgMap = createMemo((): Record<string, string> => {
    try { return JSON.parse(eCfgJson() || "{}"); } catch { return {}; }
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
                  setCRuntime(v); setCModel(""); setCMode(""); setCConfigSel({});
                  void props.fetchConfigOptions(v).then(setCConfigOpts);
                }} />
              </FieldRow>
              <FieldRow label="Prompt">
                <TextInput value={cPrompt()} onInput={setCPrompt} placeholder="System prompt…" multiline rows={4} />
              </FieldRow>
              {/* Model — dropdown if runtime provides options, otherwise plain text */}
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
              {/* Mode — only shown when runtime provides it */}
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
                  <span class="font-mono text-[10px] theme-muted">auto-approve permissions</span>
                </label>
              </FieldRow>
              {/* Other runtime-specific options (excluding model/mode) */}
              <Show when={cConfigOpts().length > 0}>
                <For each={cConfigOpts().filter((o) => !isModelOrMode(o))}>
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
            const otherOpts = createMemo(() => editConfigOpts().filter((o) => !isModelOrMode(o)));
            return (
              <div class="space-y-4">
                <div class="flex items-start justify-between gap-4">
                  <div>
                    <h2 class="font-mono text-sm font-bold theme-text">{role().roleName}</h2>
                    <span class={`font-mono text-[10px] ${RUNTIME_COLOR[role().runtimeKind] ?? "theme-muted"}`}>{role().runtimeKind}</span>
                    <span class="font-mono text-[9px] theme-muted ml-2">provider locked</span>
                  </div>
                  <Show when={deletingId() === role().roleName} fallback={
                    <ActionButton label="Delete" variant="danger" onClick={() => setDeletingId(role().roleName)} />
                  }>
                    <div class="flex items-center gap-2">
                      <ActionButton label="Confirm delete" variant="danger" onClick={() => void handleDelete(role().roleName)} />
                      <ActionButton label="Cancel" variant="ghost" onClick={() => setDeletingId(null)} />
                    </div>
                  </Show>
                </div>

                <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
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
                    <div class="space-y-2">
                      <Show when={eMcpServers().length > 0}>
                        <div class="space-y-1">
                          <For each={eMcpServers()}>
                            {(srv, idx) => {
                              const t = () => mcpTransport(srv);
                              const tColor = () => ({ stdio: "bg-amber-500/15 text-amber-300", http: "bg-sky-500/15 text-sky-300", sse: "bg-violet-500/15 text-violet-300" }[t()] ?? "");
                              return (
                                <div class="flex items-center gap-2 rounded-md border theme-border bg-[var(--ui-surface)] px-2 py-1.5">
                                  <Badge label={t()} color={tColor()} />
                                  <span class="flex-1 truncate font-mono text-[10px] theme-text">{srv.name}</span>
                                  <span class="truncate font-mono text-[9px] theme-muted max-w-[200px]">{mcpDisplayUri(srv)}</span>
                                  <button
                                    onClick={() => setEMcpServers((s) => s.filter((_, i) => i !== idx()))}
                                    class="shrink-0 theme-muted hover:text-rose-400"
                                  >
                                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                                  </button>
                                </div>
                              );
                            }}
                          </For>
                        </div>
                      </Show>
                      <Show when={eMcpAdding()} fallback={
                        <button
                          onClick={() => { setEMcpAdding(true); setEMcpName(""); setEMcpCommand(""); setEMcpArgs(""); setEMcpUrl(""); setEMcpTransport("stdio"); }}
                          class="font-mono text-[10px] theme-muted hover:theme-text"
                        >
                          + add MCP server
                        </button>
                      }>
                        <div class="space-y-1.5 rounded-md border theme-border bg-[var(--ui-surface)] p-2.5">
                          <div class="flex gap-2">
                            <input
                              value={eMcpName()} onInput={(e) => setEMcpName(e.currentTarget.value)}
                              placeholder="name" class="h-6 flex-1 rounded border theme-border theme-surface px-1.5 font-mono text-[10px] theme-text placeholder:theme-muted focus:outline-none"
                            />
                            <InlineSelect
                              value={eMcpTransport()}
                              options={[{ value: "stdio", label: "stdio" }, { value: "http", label: "http" }, { value: "sse", label: "sse" }]}
                              onChange={(v) => setEMcpTransport(v as "stdio" | "http" | "sse")}
                              class="w-24"
                            />
                          </div>
                          <Show when={eMcpTransport() === "stdio"}>
                            <input
                              value={eMcpCommand()} onInput={(e) => setEMcpCommand(e.currentTarget.value)}
                              placeholder="command (e.g. npx)" class="h-6 w-full rounded border theme-border theme-surface px-1.5 font-mono text-[10px] theme-text placeholder:theme-muted focus:outline-none"
                            />
                            <input
                              value={eMcpArgs()} onInput={(e) => setEMcpArgs(e.currentTarget.value)}
                              placeholder="args (e.g. -y @anthropic-ai/chrome-devtools-mcp@latest)" class="h-6 w-full rounded border theme-border theme-surface px-1.5 font-mono text-[10px] theme-text placeholder:theme-muted focus:outline-none"
                            />
                          </Show>
                          <Show when={eMcpTransport() !== "stdio"}>
                            <input
                              value={eMcpUrl()} onInput={(e) => setEMcpUrl(e.currentTarget.value)}
                              placeholder="url (e.g. https://mcp.example.com)" class="h-6 w-full rounded border theme-border theme-surface px-1.5 font-mono text-[10px] theme-text placeholder:theme-muted focus:outline-none"
                            />
                          </Show>
                          <div class="flex gap-2 pt-1">
                            <button
                              onClick={() => {
                                const name = eMcpName().trim();
                                if (!name) return;
                                let srv: AcpMcpServer;
                                if (eMcpTransport() === "stdio") {
                                  const cmd = eMcpCommand().trim();
                                  if (!cmd) return;
                                  const parsedArgs = parseCommandArgs(eMcpArgs().trim());
                                  if (parsedArgs === null) {
                                    props.pushMessage("event", "Invalid MCP args: check quotes/escaping.");
                                    return;
                                  }
                                  srv = { name, command: cmd, args: parsedArgs, env: [] };
                                } else {
                                  const url = eMcpUrl().trim();
                                  if (!url) return;
                                  srv = { type: eMcpTransport() as "http" | "sse", name, url, headers: [] };
                                }
                                setEMcpServers((s) => [...s, srv]);
                                setEMcpAdding(false);
                              }}
                              class="font-mono text-[10px] text-emerald-400 hover:text-emerald-300"
                            >add</button>
                            <button onClick={() => setEMcpAdding(false)} class="font-mono text-[10px] theme-muted hover:theme-text">cancel</button>
                          </div>
                        </div>
                      </Show>
                    </div>
                  </FieldRow>
                  <Show when={eGlobalMcp().length > 0}>
                    <FieldRow label="Global MCP">
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
                  <FieldRow label="Auto-approve">
                    <label class="flex items-center gap-2 cursor-pointer">
                      <input type="checkbox" checked={eAutoApprove()} onChange={(e) => setEAutoApprove(e.currentTarget.checked)} class="rounded accent-emerald-500" />
                      <span class="font-mono text-[10px] theme-muted">auto-approve permissions</span>
                    </label>
                  </FieldRow>
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
