import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { Role, RoleUpsertInput, AppSession, AssistantRuntime, AcpConfigOption, AppSkill } from "./types";
import { INTERACTIVE_MOTION, RUNTIME_COLOR, flattenConfigValues } from "./types";
import { assistantApi, configApi, roleApi } from "../lib/tauriApi";

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
  pushMessage: (role: string, text: string) => void;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<AcpConfigOption[]>;
  onOpenManagement?: (tab?: "roles" | "skills" | "sessions" | "workflows" | "mcp", roleName?: string) => void;
};

export default function ConfigDrawer(props: ConfigDrawerProps) {
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

            {/* Roles & Skills — managed in the Management panel */}
            <RolesSection roles={props.roles} onOpenManagement={props.onOpenManagement} setShowDrawer={props.setShowDrawer} />
            <div class="space-y-1.5">
              <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-600">Manage</span>
              <div class="space-y-0.5">
                {([
                  { tab: "skills" as const, label: "Skills", count: () => props.skills().length, color: "text-teal-300" },
                  { tab: "workflows" as const, label: "Workflows", count: () => null, color: "text-indigo-300" },
                  { tab: "mcp" as const, label: "MCP Registry", count: () => null, color: "text-sky-300" },
                  { tab: "sessions" as const, label: "Session History", count: () => null, color: "text-zinc-400" },
                ] as const).map(({ tab, label, count, color }) => (
                  <button
                    onClick={(e) => { e.stopPropagation(); props.setShowDrawer(false); props.onOpenManagement?.(tab); }}
                    class={`flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-left text-xs transition-colors duration-150 hover:bg-zinc-900/60 group ${INTERACTIVE_MOTION}`}
                  >
                    <span class="text-zinc-400 group-hover:text-zinc-200 transition-colors">{label}</span>
                    <div class="flex items-center gap-1.5">
                      {count() !== null && (
                        <span class={`font-mono text-[10px] ${color}`}>{count()}</span>
                      )}
                      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="text-zinc-700 group-hover:text-zinc-500 transition-colors"><path d="M5 12h14M12 5l7 7-7 7"/></svg>
                    </div>
                  </button>
                ))}
              </div>
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
  pushMessage: (role: string, text: string) => void;
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
                props.activeSession()?.runtimeKind === a.key
                  ? "bg-zinc-800/80 text-white"
                  : "hover:bg-zinc-900/60 text-zinc-400"
              } ${!a.available ? "opacity-40" : ""}`}
              onClick={() => a.available && props.patchActiveSession({ runtimeKind: a.key })}
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
  pushMessage: (role: string, text: string) => void;
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
  const [optionsLoading, setOptionsLoading] = createSignal(false);
  let optionsReqSeq = 0;

  const runtimeKind = () => props.activeSession()?.runtimeKind ?? "claude-code";

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
  const initRole = async () => {
    try {
      const roles = await roleApi.list();
      const existing = roles.find((r) => r.roleName === UNION_ASSISTANT_ROLE);
      if (existing) {
        setPrompt(existing.systemPrompt ?? "");
        setModel(existing.model ?? "");
        setMode(existing.mode ?? "");
        setConfigOptionsJson(existing.configOptionsJson || "{}");
      }
    } catch {}
  };

  const refreshOptions = async (runtime: string) => {
    const reqSeq = ++optionsReqSeq;
    const sid = props.activeSession()?.id ?? "";
    setOptionsLoading(true);
    try {
      const cachedRaw = await assistantApi.listDiscoveredConfig(runtime, UNION_ASSISTANT_ROLE, sid);
      if (reqSeq !== optionsReqSeq) return;
      setConfigOptions(configApi.asOptions(cachedRaw));
    } catch {
      if (reqSeq !== optionsReqSeq) return;
      setConfigOptions([]);
    }
    if (!sid) {
      if (reqSeq === optionsReqSeq) setOptionsLoading(false);
      return;
    }
    void assistantApi.prewarmRoleConfig(runtime, UNION_ASSISTANT_ROLE, sid).then((raw) => {
      if (reqSeq !== optionsReqSeq) return;
      setConfigOptions(configApi.asOptions(raw));
    }).catch(() => {
    }).finally(() => {
      if (reqSeq !== optionsReqSeq) return;
      setOptionsLoading(false);
    });
  };

  void initRole();
  createEffect(() => {
    const runtime = runtimeKind();
    void refreshOptions(runtime);
  });

  const handleSave = async () => {
    try {
      const saved = await roleApi.upsert({
        roleName: UNION_ASSISTANT_ROLE,
        runtimeKind: runtimeKind(),
        systemPrompt: prompt().trim(),
        model: model().trim() || null,
        mode: mode() || null,
        mcpServersJson: "[]",
        configOptionsJson: configOptionsJson(),
        autoApprove: true,
      } satisfies RoleUpsertInput);
      await props.refreshRoles();
      props.pushMessage("event", `assistant config saved (${saved.runtimeKind}${saved.model ? `, model=${saved.model}` : ""}${saved.mode ? `, mode=${saved.mode}` : ""})`);
      props.onClose();
    } catch (e) {
      props.pushMessage("event", `Failed to save config: ${String(e)}`);
    }
  };

  return (
    <div class="mt-2 space-y-2 rounded-xl border border-zinc-800/60 bg-zinc-950/60 p-3">
      <div class="flex items-center justify-between">
        <div class="text-[10px] text-zinc-600">runtime: {runtimeKind()}</div>
        <Show when={optionsLoading()}>
          <div class="text-[10px] text-zinc-600 italic">loading options…</div>
        </Show>
      </div>
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
    </div>
  );
}

function RolesSection(props: {
  roles: Accessor<Role[]>;
  onOpenManagement?: (tab?: "roles" | "skills" | "sessions" | "workflows" | "mcp", roleName?: string) => void;
  setShowDrawer: Setter<boolean>;
}) {
  const [expanded, setExpanded] = createSignal(true);
  const userRoles = () => props.roles().filter((r) => r.roleName !== "UnionAIAssistant");

  const openRole = (roleName: string) => {
    props.setShowDrawer(false);
    props.onOpenManagement?.("roles", roleName);
  };

  return (
    <div class="space-y-1">
      <button
        onClick={() => setExpanded((v) => !v)}
        class={`flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-left text-xs transition-colors duration-150 hover:bg-zinc-900/60 group ${INTERACTIVE_MOTION}`}
      >
        <span class="text-[10px] font-medium uppercase tracking-widest text-zinc-600 group-hover:text-zinc-400 transition-colors">Roles</span>
        <div class="flex items-center gap-1.5">
          <span class="font-mono text-[10px] text-orange-300">{userRoles().length}</span>
          <svg
            width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            class={`text-zinc-700 group-hover:text-zinc-500 transition-all ${expanded() ? "rotate-90" : ""}`}
          >
            <path d="M5 12h14M12 5l7 7-7 7"/>
          </svg>
        </div>
      </button>
      <Show when={expanded()}>
        <div class="space-y-0.5 pl-1">
          <Show when={userRoles().length === 0}>
            <div class="px-2.5 py-1.5 text-[10px] text-zinc-600 italic">No roles yet</div>
          </Show>
          <For each={userRoles()}>
            {(role) => {
              const color = () => RUNTIME_COLOR[role.runtimeKind] ?? "text-zinc-500";
              return (
                <button
                  onClick={(e) => { e.stopPropagation(); openRole(role.roleName); }}
                  class={`flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 hover:bg-zinc-900/40 transition-colors text-left ${INTERACTIVE_MOTION}`}
                >
                  <span class="flex-1 truncate font-mono text-[10px] text-zinc-300">{role.roleName}</span>
                  <span class={`font-mono text-[9px] shrink-0 ${color()}`}>{role.runtimeKind}</span>
                  <Show when={role.model}><span class="font-mono text-[9px] text-blue-400 shrink-0">{role.model}</span></Show>
                </button>
              );
            }}
          </For>
          <button
            onClick={(e) => { e.stopPropagation(); props.setShowDrawer(false); props.onOpenManagement?.("roles"); }}
            class={`flex w-full items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[10px] text-zinc-600 hover:text-zinc-300 hover:bg-zinc-900/40 transition-colors ${INTERACTIVE_MOTION}`}
          >
            <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg>
            manage roles
          </button>
        </div>
      </Show>
    </div>
  );
}
