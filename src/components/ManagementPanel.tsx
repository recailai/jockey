/**
 * ManagementPanel — full-screen management hub
 * Tabs: Sessions · Workflows · MCP Registry · Skill Registry
 *
 * Aesthetic: terminal-grid brutalism — tight monospace data density,
 * hairline dividers, amber/teal/indigo accent pills, zero decorative chrome.
 * The panel slides in from the right over a dimmed backdrop.
 */

import { invoke } from "@tauri-apps/api/core";
import {
  For, Show, createEffect, createMemo, createSignal, onMount,
} from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, AppSkill, Role, RoleUpsertInput, AcpConfigOption } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, RUNTIMES, flattenConfigValues } from "./types";

// ─────────────────────────────────────────────────────────────────────────────
// Domain types (backend shape — add to types.ts when wiring real invokes)
// ─────────────────────────────────────────────────────────────────────────────

type StoredSession = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  cwd: string | null;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
};

type Workflow = {
  id: string;
  name: string;
  description: string;
  steps: WorkflowStep[];
  createdAt: number;
  updatedAt: number;
  status?: "idle" | "running" | "done" | "error";
};

type WorkflowStep = {
  roleName: string;
  prompt: string;
  order: number;
};

type McpServer = {
  id: string;
  name: string;
  uri: string;
  transport: "stdio" | "http" | "sse";
  enabled: boolean;
  capabilities: string[];
  roleName?: string;
};

// ─────────────────────────────────────────────────────────────────────────────
// Primitive helpers
// ─────────────────────────────────────────────────────────────────────────────

const fmtDate = (ts: number) =>
  new Date(ts).toLocaleDateString("en-US", {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false,
  });

const fmtRelative = (ts: number) => {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared micro-components
// ─────────────────────────────────────────────────────────────────────────────

function Badge(props: { label: string; color?: string; class?: string }) {
  return (
    <span class={`inline-flex items-center rounded-sm px-1.5 py-px font-mono text-[9px] font-semibold tracking-wide uppercase ${props.color ?? "bg-zinc-800 text-zinc-400"} ${props.class ?? ""}`}>
      {props.label}
    </span>
  );
}

function EmptyState(props: { icon: string; title: string; sub?: string }) {
  return (
    <div class="flex flex-col items-center justify-center gap-2 py-16 text-center">
      <span class="text-3xl opacity-20">{props.icon}</span>
      <p class="text-xs font-medium text-zinc-500">{props.title}</p>
      {props.sub && <p class="max-w-[200px] text-[10px] text-zinc-700">{props.sub}</p>}
    </div>
  );
}

function PanelSection(props: { title: string; action?: { label: string; onClick: () => void }; children: unknown }) {
  return (
    <div>
      <div class="mb-3 flex items-center justify-between border-b border-white/[0.04] pb-2">
        <span class="font-mono text-[9px] font-semibold uppercase tracking-[0.12em] text-zinc-600">{props.title}</span>
        <Show when={props.action}>
          {(action) => (
            <button
              onClick={action().onClick}
              class={`min-h-6 rounded border border-zinc-800 px-2 py-0.5 font-mono text-[9px] text-zinc-500 hover:border-zinc-600 hover:text-zinc-200 ${INTERACTIVE_MOTION}`}
            >
              {action().label}
            </button>
          )}
        </Show>
      </div>
      {props.children as any}
    </div>
  );
}

function FieldRow(props: { label: string; children: unknown }) {
  return (
    <div class="flex items-start gap-3">
      <span class="w-20 shrink-0 font-mono text-[9px] uppercase tracking-wide text-zinc-600 pt-[3px]">{props.label}</span>
      <div class="flex-1 min-w-0">{props.children as any}</div>
    </div>
  );
}

function TextInput(props: {
  value: string; onInput: (v: string) => void; placeholder?: string;
  multiline?: boolean; rows?: number; class?: string; monospace?: boolean;
}) {
  const base = `w-full rounded-md border border-zinc-800 bg-zinc-950 px-2.5 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-700 focus:border-zinc-600 focus:outline-none ${props.monospace ? "font-mono" : ""} ${props.class ?? ""}`;
  if (props.multiline) {
    return (
      <textarea
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        placeholder={props.placeholder}
        rows={props.rows ?? 3}
        class={`${base} resize-none`}
      />
    );
  }
  return (
    <input
      value={props.value}
      onInput={(e) => props.onInput(e.currentTarget.value)}
      placeholder={props.placeholder}
      class={`h-8 ${base}`}
    />
  );
}

function InlineSelect(props: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (v: string) => void;
  class?: string;
}) {
  const [open, setOpen] = createSignal(false);
  const selected = () => props.options.find((o) => o.value === props.value);
  const close = () => setOpen(false);
  const handleOutside = (e: MouseEvent) => {
    if (!(e.target as Element).closest("[data-isel]")) close();
  };
  const toggle = () => {
    if (!open()) document.addEventListener("click", handleOutside, { once: true });
    setOpen((v) => !v);
  };
  return (
    <div data-isel class={`relative ${props.class ?? ""}`}>
      <button
        type="button"
        onClick={toggle}
        class={`flex h-8 w-full items-center justify-between gap-2 rounded-md border border-zinc-800 bg-zinc-950 px-2.5 text-xs text-left ${INTERACTIVE_MOTION} ${open() ? "border-zinc-600" : "hover:border-zinc-700"}`}
      >
        <span class={selected() ? "text-zinc-200" : "text-zinc-600"}>
          {selected()?.label ?? "Select…"}
        </span>
        <svg class={`h-3 w-3 shrink-0 text-zinc-500 transition-transform ${open() ? "rotate-180" : ""}`} viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 4l4 4 4-4" /></svg>
      </button>
      <Show when={open()}>
        <div class="absolute left-0 right-0 top-full z-50 mt-0.5 max-h-44 overflow-y-auto rounded-md border border-zinc-800 bg-[#0d0d10] shadow-xl shadow-black/50">
          <For each={props.options}>
            {(opt) => (
              <button
                type="button"
                onClick={() => { props.onChange(opt.value); close(); }}
                class={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs ${INTERACTIVE_MOTION} ${opt.value === props.value ? "bg-zinc-800/60 text-white" : "text-zinc-300 hover:bg-zinc-800/40 hover:text-white"}`}
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

function ActionButton(props: {
  onClick: () => void; label: string; variant?: "primary" | "danger" | "ghost"; class?: string;
}) {
  const cls = () => ({
    primary: "bg-zinc-100 text-zinc-900 hover:bg-white border-transparent",
    danger: "border-rose-800/60 text-rose-400 hover:bg-rose-500/10 hover:border-rose-600",
    ghost: "border-zinc-800 text-zinc-500 hover:border-zinc-600 hover:text-zinc-200",
  }[props.variant ?? "ghost"]);
  return (
    <button
      onClick={props.onClick}
      class={`min-h-7 rounded-md border px-3 text-xs font-medium transition-all duration-150 active:scale-[0.98] ${cls()} ${props.class ?? ""}`}
    >
      {props.label}
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nav tab definitions
// ─────────────────────────────────────────────────────────────────────────────

type TabId = "sessions" | "workflows" | "roles" | "mcp" | "skills";

const TABS: Array<{ id: TabId; label: string; icon: () => any }> = [
  {
    id: "sessions", label: "Sessions",
    icon: () => (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <rect x="3" y="3" width="18" height="18" rx="2" /><line x1="3" y1="9" x2="21" y2="9" /><line x1="9" y1="21" x2="9" y2="9" />
      </svg>
    ),
  },
  {
    id: "workflows", label: "Workflows",
    icon: () => (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="18" cy="5" r="3" /><circle cx="6" cy="12" r="3" /><circle cx="18" cy="19" r="3" />
        <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" /><line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
      </svg>
    ),
  },
  {
    id: "roles", label: "Roles",
    icon: () => (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" />
      </svg>
    ),
  },
  {
    id: "mcp", label: "MCP",
    icon: () => (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <polygon points="12 2 2 7 12 12 22 7 12 2" /><polyline points="2 17 12 22 22 17" /><polyline points="2 12 12 17 22 12" />
      </svg>
    ),
  },
  {
    id: "skills", label: "Skills",
    icon: () => (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
      </svg>
    ),
  },
];

// ─────────────────────────────────────────────────────────────────────────────
// Roles tab  (moved from ConfigDrawer)
// ─────────────────────────────────────────────────────────────────────────────

function RolesTab(props: {
  roles: Accessor<Role[]>;
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  refreshRoles: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  pushMessage: (role: string, text: string) => void;
}) {
  const UNION_ROLE = "UnionAIAssistant";
  const userRoles = createMemo(() => props.roles().filter((r) => r.roleName !== UNION_ROLE));

  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);

  // Create form
  const [cName, setCName] = createSignal("Developer");
  const [cRuntime, setCRuntime] = createSignal("gemini-cli");
  const [cPrompt, setCPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");
  const [cModel, setCModel] = createSignal("");
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
    props.patchActiveSession({ discoveredConfigOptions: [], configOptionsLoading: true });
    invoke<unknown[]>("prewarm_role_config_cmd", {
      runtimeKind: role.runtimeKind,
      roleName: role.roleName,
      appSessionId,
    }).then((raw) => {
      props.patchActiveSession({ discoveredConfigOptions: raw as AcpConfigOption[], configOptionsLoading: false });
    }).catch(() => props.patchActiveSession({ configOptionsLoading: false }));
  };

  const handleCreate = async () => {
    const name = cName().trim();
    if (!name) return;
    const configMap: Record<string, string> = {};
    for (const [k, v] of Object.entries(cConfigSel())) { if (v) configMap[k] = v; }
    try {
      const saved = await invoke<Role>("upsert_role_cmd", {
        input: {
          roleName: name, runtimeKind: cRuntime(),
          systemPrompt: cPrompt().trim() || "You are a helpful AI assistant.",
          model: cModel().trim() || null, mode: null,
          mcpServersJson: "[]", configOptionsJson: JSON.stringify(configMap),
          autoApprove: cAutoApprove(),
        } satisfies RoleUpsertInput,
      });
      setCreating(false);
      setCConfigSel({}); setCConfigOpts([]);
      await props.refreshRoles();
      props.pushMessage("event", `role created: ${saved.roleName} (${saved.runtimeKind})`);
    } catch (e) { props.pushMessage("event", `Failed to create role: ${String(e)}`); }
  };

  const handleSaveEdit = async () => {
    const role = editingRole();
    if (!role) return;
    let parsedMcp: unknown; let parsedCfg: unknown;
    try { parsedMcp = JSON.parse(eMcpJson().trim() || "[]"); if (!Array.isArray(parsedMcp)) throw new Error("must be array"); }
    catch (e) { props.pushMessage("event", `Invalid MCP JSON: ${String(e)}`); return; }
    try { parsedCfg = JSON.parse(eCfgJson().trim() || "{}"); if (!parsedCfg || typeof parsedCfg !== "object" || Array.isArray(parsedCfg)) throw new Error("must be object"); }
    catch (e) { props.pushMessage("event", `Invalid config JSON: ${String(e)}`); return; }
    try {
      const saved = await invoke<Role>("upsert_role_cmd", {
        input: {
          roleName: role.roleName, runtimeKind: role.runtimeKind,
          systemPrompt: ePrompt().trim(), model: eModel().trim() || null,
          mode: eMode().trim() || null, mcpServersJson: JSON.stringify(parsedMcp),
          configOptionsJson: JSON.stringify(parsedCfg), autoApprove: eAutoApprove(),
        } satisfies RoleUpsertInput,
      });
      setSelectedId(null);
      await props.refreshRoles();
      props.pushMessage("event", `role saved: ${saved.roleName}`);
    } catch (e) { props.pushMessage("event", `Failed to save: ${String(e)}`); }
  };

  const handleDelete = async (roleName: string) => {
    try {
      await invoke("delete_role_cmd", { roleName });
      if (editingRole()?.roleName === roleName) setSelectedId(null);
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
            onClick={() => { setCreating(true); setSelectedId(null); }}
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
                    <button
                      onClick={(e) => { e.stopPropagation(); void handleDelete(role.roleName); }}
                      class="shrink-0 opacity-0 group-hover:opacity-100 text-zinc-700 hover:text-rose-400 transition-all"
                    >
                      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                    </button>
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
                <TextInput value={cName()} onInput={setCName} placeholder="e.g. Developer" monospace />
              </FieldRow>
              <FieldRow label="Runtime">
                <InlineSelect value={cRuntime()} options={runtimeOptions} onChange={(v) => {
                  setCRuntime(v); setCConfigSel({});
                  void props.fetchConfigOptions(v).then(setCConfigOpts);
                }} />
              </FieldRow>
              <FieldRow label="Prompt">
                <TextInput value={cPrompt()} onInput={setCPrompt} placeholder="System prompt…" multiline rows={4} />
              </FieldRow>
              <FieldRow label="Model">
                <TextInput value={cModel()} onInput={setCModel} placeholder="Optional model override" monospace />
              </FieldRow>
              <FieldRow label="Auto-approve">
                <label class="flex items-center gap-2 cursor-pointer">
                  <input type="checkbox" checked={cAutoApprove()} onChange={(e) => setCAutoApprove(e.currentTarget.checked)} class="rounded accent-emerald-500" />
                  <span class="font-mono text-[10px] text-zinc-500">auto-approve permissions</span>
                </label>
              </FieldRow>
              <Show when={cConfigOpts().length > 0}>
                <For each={cConfigOpts()}>
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
              <ActionButton label="Create" variant="primary" onClick={() => void handleCreate()} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>
          </div>
        </Show>

        {/* Edit form */}
        <Show when={!creating() && editingRole()}>
          {(role) => {
            const modelOpt = createMemo(() => editConfigOpts().find((o) => o.category === "model" || o.id === "model"));
            const modeOpt = createMemo(() => editConfigOpts().find((o) => o.category === "mode" || o.id === "mode"));
            const otherOpts = createMemo(() => editConfigOpts().filter((o) => o.id !== "model" && o.id !== "mode" && o.category !== "model" && o.category !== "mode"));
            return (
              <div class="space-y-4">
                <div class="flex items-start justify-between gap-4">
                  <div>
                    <h2 class="font-mono text-sm font-bold text-zinc-100">{role().roleName}</h2>
                    <span class={`font-mono text-[10px] ${RUNTIME_COLOR[role().runtimeKind] ?? "text-zinc-500"}`}>{role().runtimeKind}</span>
                    <span class="font-mono text-[9px] text-zinc-700 ml-2">provider locked</span>
                  </div>
                  <ActionButton label="Delete" variant="danger" onClick={() => void handleDelete(role().roleName)} />
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
                            options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.name }))]}
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
                            options={[{ value: "", label: `default: ${mo().currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.name }))]}
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
                            options={[{ value: "", label: `default: ${opt.currentValue}` }, ...values().map((v) => ({ value: v.value, label: v.name }))]}
                            onChange={(val) => updateEditCfg(opt.id, val)}
                          />
                        </FieldRow>
                      );
                    }}
                  </For>

                  <Show when={editConfigOpts().length === 0}>
                    <p class="font-mono text-[10px] text-zinc-700 italic">
                      {props.activeSession()?.configOptionsLoading ? "Loading agent config options…" : "No config options available."}
                    </p>
                  </Show>

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
                  <ActionButton label="Save" variant="primary" onClick={() => void handleSaveEdit()} />
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

// ─────────────────────────────────────────────────────────────────────────────
// Sessions tab
// ─────────────────────────────────────────────────────────────────────────────

function SessionsTab(props: {
  activeSessions: AppSession[];
  onRestoreSession?: (id: string) => void;
}) {
  const [storedSessions, setStoredSessions] = createSignal<StoredSession[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [search, setSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      const raw = await invoke<Array<{
        id: string; title: string; activeRole?: string;
        runtimeKind?: string | null; cwd?: string | null;
        messages?: unknown[]; createdAt?: number; updatedAt?: number;
      }>>("list_app_sessions");
      setStoredSessions(raw.map((r) => ({
        id: r.id,
        title: r.title,
        activeRole: r.activeRole ?? "—",
        runtimeKind: r.runtimeKind ?? null,
        cwd: r.cwd ?? null,
        messageCount: Array.isArray(r.messages) ? r.messages.length : 0,
        createdAt: r.createdAt ?? 0,
        updatedAt: r.updatedAt ?? 0,
      })));
    } catch { /* ignore */ }
    setLoading(false);
  };

  onMount(() => void load());

  const filtered = createMemo(() => {
    const q = search().toLowerCase();
    return storedSessions().filter((s) =>
      !q || s.title.toLowerCase().includes(q) || (s.cwd ?? "").toLowerCase().includes(q),
    );
  });

  const selected = createMemo(() =>
    filtered().find((s) => s.id === selectedId()) ?? null,
  );

  const activeIds = new Set(props.activeSessions.map((s) => s.id));

  return (
    <div class="flex h-full">
      {/* List pane */}
      <div class="flex w-64 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <input
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
            placeholder="Filter sessions…"
            class="h-7 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2.5 font-mono text-[10px] text-zinc-300 placeholder:text-zinc-700 focus:border-zinc-600 focus:outline-none"
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={loading()}>
            <p class="p-4 font-mono text-[10px] text-zinc-700">Loading…</p>
          </Show>
          <Show when={!loading() && filtered().length === 0}>
            <p class="p-4 font-mono text-[10px] text-zinc-700">No sessions found.</p>
          </Show>
          <For each={filtered()}>
            {(s) => {
              const isActive = activeIds.has(s.id);
              const color = () => RUNTIME_COLOR[s.runtimeKind ?? ""] ?? "text-zinc-500";
              return (
                <button
                  onClick={() => setSelectedId(s.id)}
                  class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === s.id ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"}`}
                >
                  <div class="flex items-center gap-1.5 min-w-0">
                    <Show when={isActive}>
                      <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400 shadow-[0_0_4px_rgba(52,211,153,0.5)]" />
                    </Show>
                    <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === s.id ? "text-zinc-100" : "text-zinc-300"}`}>{s.title}</span>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <Show when={s.runtimeKind}>
                      <span class={`font-mono text-[9px] ${color()}`}>{s.runtimeKind}</span>
                      <span class="text-zinc-700">·</span>
                    </Show>
                    <span class="font-mono text-[9px] text-zinc-600">{s.messageCount} msgs</span>
                    <span class="text-zinc-700">·</span>
                    <span class="font-mono text-[9px] text-zinc-700">{fmtRelative(s.updatedAt || s.createdAt)}</span>
                  </div>
                </button>
              );
            }}
          </For>
        </div>
      </div>

      {/* Detail pane */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={selected()} fallback={
          <EmptyState icon="◫" title="Select a session" sub="Click any session on the left to inspect it" />
        }>
          {(s) => (
            <div class="space-y-5">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{s().title}</h2>
                  <p class="mt-0.5 font-mono text-[10px] text-zinc-600">{s().id}</p>
                </div>
                <div class="flex gap-2 shrink-0">
                  <Show when={activeIds.has(s().id)}>
                    <Badge label="active" color="bg-emerald-500/15 text-emerald-300" />
                  </Show>
                  <Show when={!activeIds.has(s().id) && props.onRestoreSession}>
                    <ActionButton label="Restore" variant="ghost" onClick={() => props.onRestoreSession?.(s().id)} />
                  </Show>
                </div>
              </div>

              <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                <FieldRow label="Role">
                  <span class="font-mono text-xs text-zinc-200">{s().activeRole}</span>
                </FieldRow>
                <FieldRow label="Runtime">
                  <span class={`font-mono text-xs ${RUNTIME_COLOR[s().runtimeKind ?? ""] ?? "text-zinc-500"}`}>
                    {s().runtimeKind ?? "—"}
                  </span>
                </FieldRow>
                <FieldRow label="Directory">
                  <span class="break-all font-mono text-[10px] text-zinc-400">{s().cwd ?? "—"}</span>
                </FieldRow>
                <FieldRow label="Messages">
                  <span class="font-mono text-xs text-zinc-200">{s().messageCount}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] text-zinc-500">{s().createdAt ? fmtDate(s().createdAt) : "—"}</span>
                </FieldRow>
                <FieldRow label="Updated">
                  <span class="font-mono text-[10px] text-zinc-500">{s().updatedAt ? fmtDate(s().updatedAt) : "—"}</span>
                </FieldRow>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Workflows tab
// ─────────────────────────────────────────────────────────────────────────────

function WorkflowsTab(props: { roles: Role[] }) {
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
      const raw = await invoke<Workflow[]>("list_workflows");
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
      await invoke("create_workflow", { name, description: wfDesc().trim(), steps: wfSteps() });
      setCreating(false);
      setWfName(""); setWfDesc("");
      setWfSteps([{ roleName: "", prompt: "", order: 0 }]);
      await load();
    } catch { /* TODO: error toast */ }
  };

  const handleDelete = async (id: string) => {
    try {
      await invoke("delete_workflow", { id });
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
      <div class="flex w-56 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <ActionButton
            label="+ New Workflow"
            variant="ghost"
            class="w-full justify-center text-center"
            onClick={() => { setCreating(true); setSelectedId(null); }}
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={loading()}>
            <p class="p-4 font-mono text-[10px] text-zinc-700">Loading…</p>
          </Show>
          <Show when={!loading() && workflows().length === 0}>
            <EmptyState icon="⬡" title="No workflows" sub="Create your first workflow" />
          </Show>
          <For each={workflows()}>
            {(wf) => (
              <button
                onClick={() => { setSelectedId(wf.id); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === wf.id ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"}`}
              >
                <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === wf.id ? "text-zinc-100" : "text-zinc-300"}`}>{wf.name}</span>
                <div class="flex items-center gap-1.5">
                  <span class="font-mono text-[9px] text-zinc-600">{wf.steps?.length ?? 0} steps</span>
                  <span class="text-zinc-700">·</span>
                  <span class="font-mono text-[9px] text-zinc-700">{fmtRelative(wf.updatedAt || wf.createdAt)}</span>
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
            <h3 class="font-mono text-xs font-bold text-zinc-300 uppercase tracking-widest">New Workflow</h3>
            <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
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
                    <div class="rounded-lg border border-white/[0.04] bg-zinc-950/30 p-3 space-y-2">
                      <div class="flex items-center justify-between">
                        <span class="font-mono text-[9px] text-zinc-600 uppercase tracking-widest">Step {i() + 1}</span>
                        <Show when={wfSteps().length > 1}>
                          <button
                            onClick={() => removeStep(i())}
                            class="font-mono text-[9px] text-zinc-700 hover:text-rose-400 transition-colors"
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
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{wf().name}</h2>
                  <Show when={wf().description}>
                    <p class="mt-1 text-[10px] text-zinc-500">{wf().description}</p>
                  </Show>
                </div>
                <ActionButton label="Delete" variant="danger" onClick={() => void handleDelete(wf().id)} />
              </div>

              <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                <FieldRow label="ID">
                  <span class="font-mono text-[10px] text-zinc-600">{wf().id}</span>
                </FieldRow>
                <FieldRow label="Created">
                  <span class="font-mono text-[10px] text-zinc-500">{fmtDate(wf().createdAt)}</span>
                </FieldRow>
              </div>

              <PanelSection title={`Steps (${wf().steps?.length ?? 0})`}>
                <div class="space-y-2">
                  <Show when={(wf().steps?.length ?? 0) === 0}>
                    <p class="font-mono text-[10px] text-zinc-700">No steps defined.</p>
                  </Show>
                  <For each={wf().steps ?? []}>
                    {(step, i) => (
                      <div class="rounded-lg border border-white/[0.04] bg-zinc-950/30 p-3">
                        <div class="mb-1.5 flex items-center gap-2">
                          <span class="font-mono text-[9px] text-zinc-700 uppercase tracking-widest">Step {i() + 1}</span>
                          <span class={`font-mono text-[9px] font-semibold ${RUNTIME_COLOR[props.roles.find((r) => r.roleName === step.roleName)?.runtimeKind ?? ""] ?? "text-zinc-400"}`}>
                            {step.roleName || "—"}
                          </span>
                        </div>
                        <p class="text-[10px] text-zinc-400 leading-relaxed">{step.prompt}</p>
                      </div>
                    )}
                  </For>
                </div>
              </PanelSection>
            </div>
          )}
        </Show>

        <Show when={!creating() && !selected()}>
          <EmptyState icon="⬡" title="Select a workflow" sub="Or create a new one" />
        </Show>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// MCP Registry tab
// ─────────────────────────────────────────────────────────────────────────────

function McpRegistryTab(props: { roles: Role[] }) {
  const [servers, setServers] = createSignal<McpServer[]>([]);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);

  // Form state
  const [fName, setFName] = createSignal("");
  const [fUri, setFUri] = createSignal("");
  const [fTransport, setFTransport] = createSignal<"stdio" | "http" | "sse">("stdio");
  const [fRole, setFRole] = createSignal("");

  // Seed from existing roles' mcpServersJson on mount
  onMount(() => {
    const discovered: McpServer[] = [];
    props.roles.forEach((role) => {
      try {
        const arr = JSON.parse(role.mcpServersJson || "[]") as Array<{ name?: string; uri?: string; url?: string; transport?: string }>;
        arr.forEach((entry, idx) => {
          discovered.push({
            id: `${role.id}-mcp-${idx}`,
            name: entry.name ?? `${role.roleName}#${idx}`,
            uri: entry.uri ?? entry.url ?? "",
            transport: (entry.transport as "stdio" | "http" | "sse") ?? "stdio",
            enabled: true,
            capabilities: [],
            roleName: role.roleName,
          });
        });
      } catch { /* ignore */ }
    });
    setServers(discovered);
    if (discovered.length > 0) setSelectedId(discovered[0].id);
  });

  const selected = createMemo(() => servers().find((s) => s.id === selectedId()) ?? null);

  const transportBadge = (t: string) => ({
    stdio: "bg-amber-500/15 text-amber-300",
    http: "bg-sky-500/15 text-sky-300",
    sse: "bg-violet-500/15 text-violet-300",
  }[t] ?? "bg-zinc-800 text-zinc-400");

  const handleAdd = () => {
    const name = fName().trim();
    if (!name || !fUri().trim()) return;
    const newServer: McpServer = {
      id: `local-${Date.now()}`,
      name,
      uri: fUri().trim(),
      transport: fTransport(),
      enabled: true,
      capabilities: [],
      roleName: fRole() || undefined,
    };
    setServers((s) => [...s, newServer]);
    setSelectedId(newServer.id);
    setCreating(false);
    setFName(""); setFUri(""); setFTransport("stdio"); setFRole("");
  };

  const toggleEnabled = (id: string) =>
    setServers((s) => s.map((srv) => srv.id === id ? { ...srv, enabled: !srv.enabled } : srv));

  const removeServer = (id: string) => {
    setServers((s) => s.filter((srv) => srv.id !== id));
    if (selectedId() === id) setSelectedId(null);
  };

  const roleOptions = createMemo(() =>
    [{ value: "", label: "— Global (all roles) —" }, ...props.roles.map((r) => ({ value: r.roleName, label: r.roleName }))]
  );

  return (
    <div class="flex h-full">
      {/* List */}
      <div class="flex w-60 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <ActionButton
            label="+ Register Server"
            variant="ghost"
            class="w-full"
            onClick={() => { setCreating(true); setSelectedId(null); }}
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={servers().length === 0}>
            <EmptyState icon="◈" title="No MCP servers" sub="Servers are imported from role configurations" />
          </Show>
          <For each={servers()}>
            {(srv) => (
              <button
                onClick={() => { setSelectedId(srv.id); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === srv.id ? "bg-zinc-800/50" : "hover:bg-zinc-900/50"} ${!srv.enabled ? "opacity-40" : ""}`}
              >
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${srv.enabled ? "bg-emerald-400" : "bg-zinc-700"}`} />
                  <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === srv.id ? "text-zinc-100" : "text-zinc-300"}`}>{srv.name}</span>
                </div>
                <div class="flex items-center gap-1.5 pl-3">
                  <Badge label={srv.transport} color={transportBadge(srv.transport)} />
                  <Show when={srv.roleName}>
                    <span class="font-mono text-[9px] text-zinc-600 truncate">{srv.roleName}</span>
                  </Show>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>

      {/* Detail */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={creating()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold text-zinc-300 uppercase tracking-widest">Register MCP Server</h3>
            <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
              <FieldRow label="Name">
                <TextInput value={fName()} onInput={setFName} placeholder="e.g. filesystem" />
              </FieldRow>
              <FieldRow label="URI">
                <TextInput value={fUri()} onInput={setFUri} placeholder="npx @modelcontextprotocol/server-filesystem" monospace />
              </FieldRow>
              <FieldRow label="Transport">
                <InlineSelect
                  value={fTransport()}
                  options={[
                    { value: "stdio", label: "stdio — subprocess" },
                    { value: "http", label: "http — HTTP/JSON-RPC" },
                    { value: "sse", label: "sse — Server-Sent Events" },
                  ]}
                  onChange={(v) => setFTransport(v as "stdio" | "http" | "sse")}
                />
              </FieldRow>
              <FieldRow label="Role">
                <InlineSelect value={fRole()} options={roleOptions()} onChange={setFRole} />
              </FieldRow>
            </div>
            <div class="flex gap-2">
              <ActionButton label="Add" variant="primary" onClick={handleAdd} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>
          </div>
        </Show>

        <Show when={!creating() && selected()}>
          {(srv) => (
            <div class="space-y-5">
              <div class="flex items-start justify-between gap-4">
                <div class="flex items-center gap-2">
                  <span class={`h-2 w-2 rounded-full ${srv().enabled ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.4)]" : "bg-zinc-700"}`} />
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{srv().name}</h2>
                  <Badge label={srv().transport} color={transportBadge(srv().transport)} />
                </div>
                <div class="flex gap-2 shrink-0">
                  <ActionButton
                    label={srv().enabled ? "Disable" : "Enable"}
                    variant="ghost"
                    onClick={() => toggleEnabled(srv().id)}
                  />
                  <ActionButton label="Remove" variant="danger" onClick={() => removeServer(srv().id)} />
                </div>
              </div>

              <div class="space-y-2 rounded-lg border border-white/[0.04] bg-zinc-950/40 p-4">
                <FieldRow label="URI">
                  <span class="break-all font-mono text-[10px] text-zinc-300">{srv().uri}</span>
                </FieldRow>
                <FieldRow label="Transport">
                  <Badge label={srv().transport} color={transportBadge(srv().transport)} />
                </FieldRow>
                <FieldRow label="Scope">
                  <span class="font-mono text-[10px] text-zinc-400">{srv().roleName ?? "global"}</span>
                </FieldRow>
                <FieldRow label="Status">
                  <div class="flex items-center gap-1.5">
                    <span class={`h-1.5 w-1.5 rounded-full ${srv().enabled ? "bg-emerald-400" : "bg-zinc-700"}`} />
                    <span class="font-mono text-[10px] text-zinc-400">{srv().enabled ? "enabled" : "disabled"}</span>
                  </div>
                </FieldRow>
              </div>

              <Show when={(srv().capabilities?.length ?? 0) > 0}>
                <PanelSection title="Capabilities">
                  <div class="flex flex-wrap gap-1.5">
                    <For each={srv().capabilities}>
                      {(cap) => <Badge label={cap} color="bg-teal-500/15 text-teal-300" />}
                    </For>
                  </div>
                </PanelSection>
              </Show>

              <div class="rounded-lg border border-amber-500/10 bg-amber-500/5 p-3">
                <p class="font-mono text-[10px] text-amber-600/80 leading-relaxed">
                  MCP server configuration is stored in the role's <code class="text-amber-400">mcpServersJson</code> field.
                  Switch to the Roles tab to edit it directly.
                </p>
              </div>
            </div>
          )}
        </Show>

        <Show when={!creating() && !selected()}>
          <EmptyState icon="◈" title="Select an MCP server" sub="Or register a new one" />
        </Show>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Skill Registry tab
// ─────────────────────────────────────────────────────────────────────────────

function SkillRegistryTab(props: {
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
        await invoke("upsert_app_skill", { input: { id: selectedId(), ...payload } });
      } catch { /* ignore */ }
    } else {
      try {
        await invoke("upsert_app_skill", { input: payload });
      } catch { /* ignore */ }
    }
    await props.refreshSkills();
    setCreating(false); setEditing(false);
  };

  const handleDelete = async (id: string) => {
    try {
      await invoke("delete_app_skill", { id });
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

// ─────────────────────────────────────────────────────────────────────────────
// Root ManagementPanel
// ─────────────────────────────────────────────────────────────────────────────

export type ManagementPanelProps = {
  show: Accessor<boolean>;
  onClose: () => void;
  initialTab?: TabId;
  activeSessions: AppSession[];
  skills: Accessor<AppSkill[]>;
  roles: Accessor<Role[]>;
  refreshSkills: () => Promise<void>;
  // Roles tab extras
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  refreshRoles: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  pushMessage: (role: string, text: string) => void;
};

export default function ManagementPanel(props: ManagementPanelProps) {
  const [activeTab, setActiveTab] = createSignal<TabId>(props.initialTab ?? "sessions");

  // Sync initial tab if parent changes it while open
  createEffect(() => {
    if (props.show() && props.initialTab) setActiveTab(props.initialTab);
  });

  const handleBackdrop = (e: MouseEvent) => {
    if ((e.target as Element).closest("[data-panel]")) return;
    props.onClose();
  };

  return (
    <Show when={props.show()}>
      {/* Backdrop */}
      <div
        class="absolute inset-0 z-50 flex items-stretch bg-black/40 backdrop-blur-[2px]"
        onClick={handleBackdrop}
      >
        {/* Panel */}
        <div
          data-panel
          class="ml-auto flex h-full w-[760px] max-w-[92vw] flex-col bg-[#0b0b0e] border-l border-white/[0.05] shadow-2xl shadow-black/60"
          style="animation: slideInRight 180ms cubic-bezier(0.16,1,0.3,1) both"
        >
          {/* Top bar */}
          <div class="flex h-11 shrink-0 items-center border-b border-white/[0.04] bg-[#0a0a0c]/80 backdrop-blur-md">
            {/* Nav tabs */}
            <div class="flex items-stretch h-full">
              <For each={TABS}>
                {(tab) => (
                  <button
                    onClick={() => setActiveTab(tab.id)}
                    class={`flex h-full items-center gap-1.5 border-b-[1.5px] px-4 font-mono text-[10px] font-semibold uppercase tracking-widest transition-colors duration-150 ${
                      activeTab() === tab.id
                        ? "border-zinc-300 text-zinc-200"
                        : "border-transparent text-zinc-600 hover:text-zinc-400 hover:border-zinc-700"
                    }`}
                  >
                    <span class={activeTab() === tab.id ? "text-zinc-300" : "text-zinc-700"}>
                      {tab.icon()}
                    </span>
                    {tab.label}
                  </button>
                )}
              </For>
            </div>

            <div class="flex-1" />

            <button
              onClick={props.onClose}
              class={`mr-3 flex h-7 w-7 items-center justify-center rounded-md text-zinc-600 hover:bg-white/[0.05] hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
              title="Close (Cmd+K)"
            >
              <svg viewBox="0 0 12 12" class="h-3 w-3" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M1 1l10 10M11 1L1 11" />
              </svg>
            </button>
          </div>

          {/* Tab content */}
          <div class="flex-1 overflow-hidden">
            <Show when={activeTab() === "sessions"}>
              <SessionsTab activeSessions={props.activeSessions} />
            </Show>
            <Show when={activeTab() === "workflows"}>
              <WorkflowsTab roles={props.roles()} />
            </Show>
            <Show when={activeTab() === "roles"}>
              <RolesTab
                roles={props.roles}
                activeSession={props.activeSession}
                patchActiveSession={props.patchActiveSession}
                refreshRoles={props.refreshRoles}
                fetchConfigOptions={props.fetchConfigOptions}
                pushMessage={props.pushMessage}
              />
            </Show>
            <Show when={activeTab() === "mcp"}>
              <McpRegistryTab roles={props.roles()} />
            </Show>
            <Show when={activeTab() === "skills"}>
              <SkillRegistryTab skills={props.skills()} refreshSkills={props.refreshSkills} />
            </Show>
          </div>
        </div>
      </div>

      {/* Keyframe — injected once */}
      <style>{`
        @keyframes slideInRight {
          from { opacity: 0; transform: translateX(24px); }
          to   { opacity: 1; transform: translateX(0); }
        }
      `}</style>
    </Show>
  );
}
