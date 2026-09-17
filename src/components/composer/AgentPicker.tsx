import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { Check, ChevronDown, ChevronRight } from "lucide-solid";
import type { Accessor } from "solid-js";
import type { AcpConfigOption, AppSession, AssistantRuntime, Role } from "../types";
import { DEFAULT_ROLE_ALIAS, INTERACTIVE_MOTION, RUNTIME_COLOR } from "../types";
import {
  appliesToModel,
  effectiveDefault,
  findModelOption,
  otherRuntimeOptions,
  readRuntimeOptions,
  storedValue,
  valuesForModel,
  type RuntimeOption,
  type RuntimeOptionValue,
} from "../../lib/runtimeOptions";
import { projectAgentApi } from "../../lib/tauriApi";
import {
  DropdownContent,
  DropdownItem,
  DropdownLabel,
  DropdownMenu,
  DropdownSeparator,
  DropdownSub,
  DropdownSubContent,
  DropdownSubTrigger,
  DropdownTrigger,
} from "../ui";

type AgentPickerProps = {
  activeSession: Accessor<AppSession | null>;
  assistants: Accessor<AssistantRuntime[]>;
  roles: Accessor<Role[]>;
  configOptions: Accessor<AcpConfigOption[]>;
  onSelectAgent: (runtimeKind: string, profileId: string | null) => void;
  onSelectRole: (roleName: string) => void;
  onManagePersonas?: () => void;
};

/** Titles the well-known reasoning levels; anything else shows the runtime's own wording. */
const VALUE_LABEL: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  off: "Off",
  minimal: "Minimal",
  ultra: "Ultra",
};

const label = (value: string) => VALUE_LABEL[value] ?? value;

function SummaryRow(props: { label: string; value: string; unset?: boolean }) {
  return (
    <div class="jui-agent-row">
      <span class="jui-agent-row-label">{props.label}</span>
      <span class="jui-agent-row-value" classList={{ "is-unset": props.unset }}>
        {props.value}
      </span>
      <ChevronRight size={13} class="jui-agent-row-chevron" />
    </div>
  );
}

function ToggleRow(props: { label: string; on: boolean }) {
  return (
    <div class="jui-agent-row">
      <span class="jui-agent-row-label">{props.label}</span>
      <span class="jui-agent-toggle" classList={{ "is-on": props.on }} />
    </div>
  );
}

export default function AgentPicker(props: AgentPickerProps) {
  const [open, setOpen] = createSignal(false);
  const [config, setConfig] = createSignal<Record<string, string>>({});
  const [pins, setPins] = createSignal<Record<string, string>>({});

  const projectId = () => props.activeSession()?.projectId ?? null;
  const sessionId = () => props.activeSession()?.id ?? null;
  const runtimeKind = () => props.activeSession()?.runtimeKind ?? null;
  const activeRole = () => props.activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS;

  const agent = createMemo(() => props.assistants().find((a) => a.key === runtimeKind()));

  // The project layer decides what the next turn actually runs with, so the panel reads from
  // it rather than from the persona default.
  createEffect(() => {
    const pid = projectId();
    const role = activeRole();
    if (!role) {
      setConfig({});
      return;
    }
    void projectAgentApi
      .getConfig(pid, role)
      .then((res) => {
        setConfig(res.configOptions ?? {});
        // The project's pinned engine wins over whatever the session was last left on, so
        // switching persona visibly moves the Agent row too.
        const pinned = res.runtimeKind;
        if (pinned && pinned !== runtimeKind()) {
          const assistant = props.assistants().find((a) => a.key === pinned);
          props.onSelectAgent(pinned, assistant?.profileId ?? null);
          // Backend config/model discovery reads the per-session-role binding, not the
          // project pin — without this, a stale binding from an earlier manual agent
          // switch keeps serving the old runtime's model list under the new displayed agent.
          // A draft session has no row to bind yet (and no stale binding to correct) — it
          // resolves fresh once `ensureSessionPersisted` creates the row at send time.
          const sid = sessionId();
          if (sid && props.activeSession()?.persisted) {
            void projectAgentApi.bindSessionAgent(sid, role, pinned).catch(() => {});
          }
        }
      })
      .catch(() => setConfig({}));
  });

  // A persona's template runtime is only its seed; the project pin is what actually runs.
  // Labelling the list with the template made this menu contradict the Agent row above it.
  createEffect(() => {
    const pid = projectId();
    void projectAgentApi
      .listPins(pid)
      .then(setPins)
      .catch(() => setPins({}));
  });

  const effectiveRuntimeFor = (role: Role) => pins()[role.roleName] ?? role.runtimeKind;

  /** Everything the runtime declared. Both the rows below and their values come from here —
   *  the picker knows that a model is special and that a parameter is a select or a toggle,
   *  and nothing else about any particular runtime. */
  const declared = createMemo(() => readRuntimeOptions(props.configOptions()));
  const models = createMemo(() => findModelOption(declared())?.values ?? []);
  const defaultModel = createMemo(() => models().find((m) => m.isDefault));

  /** What the persona itself was configured with in Settings. That row is the seed the
   *  backend's own resolution falls back to (`resolve_model`), so the picker has to consult it
   *  before the runtime default — otherwise a freshly created persona shows the engine's
   *  default here while Settings shows what the user actually chose, and they disagree until
   *  something writes a project pin. */
  const personaTemplate = createMemo(() => {
    const role = props.roles().find((r) => r.roleName === activeRole());
    if (!role) return { model: "", byId: {} as Record<string, string> };
    let cfg: Record<string, string> = {};
    try {
      const parsed = JSON.parse(role.configOptionsJson || "{}");
      if (parsed && typeof parsed === "object") cfg = parsed as Record<string, string>;
    } catch { /* a malformed row must not blank the picker */ }
    return { model: role.model || cfg.model || "", byId: cfg };
  });
  const currentModelId = () => config().model ?? "";
  const currentModel = createMemo(() =>
    models().find((m) => m.value === currentModelId()),
  );

  /** The model this persona will actually run on: the pinned one, else the runtime's own
   *  default. Everything the catalog declares per model — capability toggles, effort levels —
   *  has to key off this. Keying off the pinned id alone hid the 1M and Fast toggles from any
   *  persona that had not pinned a model, even though the default model advertises them. */
  const effectiveModel = createMemo(() => {
    const templateId = personaTemplate().model;
    return (
      currentModel() ??
      (templateId ? models().find((m) => m.value === templateId) : undefined) ??
      defaultModel()
    );
  });

  /** Every declared parameter except the model, filtered to those the model in effect can
   *  honour. Which knobs exist is the runtime's statement, not the picker's knowledge — it
   *  never learns that Claude has a fast mode or that Codex calls its levels something else. */
  const parameters = createMemo(() => {
    const model = effectiveModel()?.value ?? "";
    return otherRuntimeOptions(declared()).filter((option) => appliesToModel(option, model));
  });

  /** The persona's own value for a parameter, or "" when it defers to the runtime. */
  const valueOf = (option: RuntimeOption) => storedValue(config(), option);
  /** What runs when the persona defers: the persona template first, then the runtime. */
  const defaultOf = (option: RuntimeOption) =>
    storedValue(personaTemplate().byId, option) || effectiveDefault(option, effectiveModel());
  const choicesOf = (option: RuntimeOption): RuntimeOptionValue[] =>
    valuesForModel(option, effectiveModel());
  const toggleOn = (option: RuntimeOption) =>
    (valueOf(option) || defaultOf(option)) === "true";
  const defaultToggleOn = (option: RuntimeOption) => defaultOf(option) === "true";

  const toggle = async (option: RuntimeOption) => {
    const next = !toggleOn(option);
    // Clearing an override resumes the role/runtime default. A literal false is needed when
    // that default is true; otherwise a project could never turn off a role-level toggle.
    await apply(option.id, next === defaultToggleOn(option) ? "" : String(next));
  };

  const agentGroups = createMemo(() => {
    const available = props.assistants().filter((a) => a.available);
    return [
      { label: "Native CLI", items: available.filter((a) => a.family === "native") },
      { label: "ACP Bridge", items: available.filter((a) => a.family !== "native") },
    ].filter((g) => g.items.length > 0);
  });

  const personaRoles = createMemo(() => {
    const seen = new Set<string>();
    return props.roles().filter((r) => {
      if (seen.has(r.roleName)) return false;
      seen.add(r.roleName);
      return true;
    });
  });

  const apply = async (configId: string, value: string) => {
    const role = activeRole();
    if (!role) return;
    try {
      // Knobs are remembered per engine, so the write has to say which engine it is for —
      // otherwise a value set while switching lands on the previous engine's setup.
      const res = await projectAgentApi.setConfig(
        projectId(),
        role,
        configId,
        value,
        runtimeKind(),
      );
      setConfig(res.configOptions ?? {});
    } catch {
      /* keep the last known value rather than rendering a half-applied state */
    }
  };

  const selectModel = async (value: string) => {
    await apply("model", value);
    // Capabilities are per model: a setting the new model cannot honour must not linger, or
    // the summary would claim something the CLI silently drops. Both checks read the
    // declaration, so a parameter added by a runtime is cleaned up without changes here.
    const next = models().find((m) => m.value === value);
    for (const option of otherRuntimeOptions(declared())) {
      const stored = storedValue(config(), option);
      if (!stored) continue;
      if (!appliesToModel(option, value)) {
        await apply(option.id, "");
        continue;
      }
      const allowed = valuesForModel(option, next);
      if (
        option.kind === "select" &&
        allowed.length > 0 &&
        !allowed.some((v) => v.value === stored)
      ) {
        await apply(option.id, "");
      }
    }
  };

  const selectAgent = (assistant: AssistantRuntime) => {
    const sid = sessionId();
    const role = activeRole();
    props.onSelectAgent(assistant.key, assistant.profileId ?? null);
    void projectAgentApi
      .setRuntime(projectId(), role, assistant.key)
      .then((res) => setConfig(res.configOptions ?? {}))
      .catch(() => {});
    if (sid && props.activeSession()?.persisted) {
      void projectAgentApi.bindSessionAgent(sid, role, assistant.key).catch(() => {});
    }
  };

  const modelSummary = () => currentModel()?.name ?? currentModelId();
  /** Unpinned rows show what will actually run, not an opaque "default: runtime". */
  const effectiveModelLabel = () => effectiveModel()?.name ?? personaTemplate().model;

  /** The chip beside the agent name: the first parameter carrying a value worth showing. */
  const summaryChip = createMemo(() => {
    for (const option of parameters()) {
      if (option.kind !== "select") continue;
      const value = valueOf(option);
      if (value) return value;
    }
    return "";
  });

  return (
    <DropdownMenu open={open()} onOpenChange={setOpen}>
      <DropdownTrigger
        variant="plain"
        class={`role-switcher ${INTERACTIVE_MOTION}`}
        title="Agent, model and effort for this project"
      >
        <span class="composer-role-dot" />
        <span class={`truncate ${RUNTIME_COLOR[runtimeKind() ?? ""] ?? ""}`}>
          {agent()?.label ?? runtimeKind() ?? "no agent"}
        </span>
        <Show when={modelSummary()}>
          <span class="shrink-0 truncate font-mono text-[10px] theme-muted">{modelSummary()}</span>
        </Show>
        <Show when={summaryChip()}>
          <span class="shrink-0 font-mono text-[10px] text-amber-300">{summaryChip()}</span>
        </Show>
        <ChevronDown size={12} class="theme-muted" />
      </DropdownTrigger>

      <DropdownContent placement="top-start" class="jui-agent-menu">
        <div class="jui-agent-header">
          <span class={`jui-agent-header-name ${RUNTIME_COLOR[runtimeKind() ?? ""] ?? ""}`}>
            {agent()?.label ?? "No agent"}
          </span>
          <span class="jui-agent-header-key">{runtimeKind() ?? "—"}</span>
        </div>

        <DropdownSub>
          <DropdownSubTrigger>
            <SummaryRow label="Persona" value={activeRole()} />
          </DropdownSubTrigger>
          <DropdownSubContent>
            <For each={personaRoles()}>
              {(role) => (
                <DropdownItem
                  class={role.roleName === activeRole() ? "is-active" : ""}
                  onSelect={() => props.onSelectRole(role.roleName)}
                >
                  <span class="jui-agent-value-row">
                    <span class="truncate">{role.roleName}</span>
                    <Show when={effectiveRuntimeFor(role)}>
                      <span
                        class={`jui-agent-value-key ${RUNTIME_COLOR[effectiveRuntimeFor(role)] ?? ""}`}
                      >
                        {effectiveRuntimeFor(role)}
                      </span>
                    </Show>
                    <Show when={role.roleName === activeRole()}>
                      <Check size={12} />
                    </Show>
                  </span>
                </DropdownItem>
              )}
            </For>
            <Show when={props.onManagePersonas}>
              <DropdownSeparator />
              <DropdownItem onSelect={() => props.onManagePersonas?.()}>
                <span class="theme-muted">Manage personas…</span>
              </DropdownItem>
            </Show>
          </DropdownSubContent>
        </DropdownSub>
        <DropdownSeparator />
        <DropdownSub>
          <DropdownSubTrigger>
            <SummaryRow label="Agent" value={agent()?.label ?? runtimeKind() ?? "none"} />
          </DropdownSubTrigger>
          <DropdownSubContent>
            <For each={agentGroups()}>
              {(group) => (
                <>
                  <DropdownLabel>{group.label}</DropdownLabel>
                  <For each={group.items}>
                    {(assistant) => (
                      <DropdownItem
                        class={assistant.key === runtimeKind() ? "is-active" : ""}
                        onSelect={() => selectAgent(assistant)}
                      >
                        <span class="jui-agent-value-row">
                          <span class="truncate">{assistant.label}</span>
                          <span class="jui-agent-value-key">{assistant.key}</span>
                          <Show when={assistant.key === runtimeKind()}>
                            <Check size={12} />
                          </Show>
                        </span>
                      </DropdownItem>
                    )}
                  </For>
                </>
              )}
            </For>
          </DropdownSubContent>
        </DropdownSub>

        <Show when={models().length > 0}>
          <DropdownSub>
            <DropdownSubTrigger>
              <SummaryRow
                label="Model"
                value={modelSummary() || effectiveModelLabel() || "default: runtime"}
                unset={!currentModelId()}
              />
            </DropdownSubTrigger>
            <DropdownSubContent>
              <DropdownItem
                class={currentModelId() ? "" : "is-active"}
                closeOnSelect={false}
                onSelect={() => void selectModel("")}
              >
                <span class="theme-muted">
                  {effectiveModelLabel() ? `default: ${effectiveModelLabel()}` : "default: runtime"}
                </span>
              </DropdownItem>
              <For each={models()}>
                {(model) => (
                  <DropdownItem
                    class={model.value === currentModelId() ? "is-active" : ""}
                    closeOnSelect={false}
                    onSelect={() => void selectModel(model.value)}
                    title={model.value}
                  >
                    <span class="jui-agent-value-row">
                      <span class="truncate">{model.name}</span>
                      <Show when={model.description}>
                        <span class="jui-agent-value-blurb">{model.description}</span>
                      </Show>
                      <Show when={model.value === currentModelId()}>
                        <Check size={12} />
                      </Show>
                    </span>
                  </DropdownItem>
                )}
              </For>
            </DropdownSubContent>
          </DropdownSub>
        </Show>

        <For each={parameters()}>
          {(option) => (
            <Show
              when={option.kind === "select"}
              fallback={
                  <DropdownItem
                    closeOnSelect={false}
                  onSelect={() => void toggle(option)}
                  title={option.description || undefined}
                >
                  <ToggleRow label={option.name} on={toggleOn(option)} />
                </DropdownItem>
              }
            >
              <Show when={choicesOf(option).length > 0}>
                <DropdownSub>
                  <DropdownSubTrigger>
                    <SummaryRow
                      label={option.name}
                      value={label(valueOf(option) || defaultOf(option)) || "default: runtime"}
                      unset={!valueOf(option)}
                    />
                  </DropdownSubTrigger>
                  <DropdownSubContent>
                    <DropdownItem
                      class={valueOf(option) ? "" : "is-active"}
                      closeOnSelect={false}
                      onSelect={() => void apply(option.id, "")}
                    >
                      <span class="theme-muted">
                        {defaultOf(option)
                          ? `default: ${label(defaultOf(option))}`
                          : "default: runtime"}
                      </span>
                    </DropdownItem>
                    <For each={choicesOf(option)}>
                      {(choice) => (
                        <DropdownItem
                          class={choice.value === valueOf(option) ? "is-active" : ""}
                          closeOnSelect={false}
                          onSelect={() => void apply(option.id, choice.value)}
                          title={choice.description || undefined}
                        >
                          <span class="jui-agent-value-row">
                            <span class="truncate">{label(choice.value) || choice.name}</span>
                            <Show when={choice.description}>
                              <span class="jui-agent-value-blurb">{choice.description}</span>
                            </Show>
                            <Show when={choice.value === valueOf(option)}>
                              <Check size={12} class="ml-auto" />
                            </Show>
                          </span>
                        </DropdownItem>
                      )}
                    </For>
                  </DropdownSubContent>
                </DropdownSub>
              </Show>
            </Show>
          )}
        </For>

      </DropdownContent>
    </DropdownMenu>
  );
}
