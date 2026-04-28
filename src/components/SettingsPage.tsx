import { For, Show, createEffect, createSignal } from "solid-js";
import {
  ArrowLeft,
  Bot,
  Boxes,
  BriefcaseBusiness,
  CheckCircle2,
  Code2,
  FolderArchive,
  GitBranch,
  Monitor,
  Palette,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Workflow,
} from "lucide-solid";
import type { Accessor } from "solid-js";
import type { AppSession, AppSkill, AssistantRuntime, Role, AcpConfigOption } from "./types";
import { type UiTheme, UI_THEMES } from "../lib/theme";
import { SessionsTab } from "./management/SessionsTab";
import { WorkflowsTab } from "./management/WorkflowsTab";
import { RolesTab } from "./management/RolesTab";
import { McpRegistryTab } from "./management/McpRegistryTab";
import { SkillRegistryTab } from "./management/SkillRegistryTab";
import { RulesTab } from "./management/RulesTab";
import { ExternalAgentsTab } from "./management/ExternalAgentsTab";
import { Panel, PanelBody, RowButton, Switch as UiSwitch } from "./ui";

export type SettingsTab =
  | "general"
  | "appearance"
  | "configuration"
  | "personalization"
  | "mcp"
  | "git"
  | "environments"
  | "worktrees"
  | "automations"
  | "roles"
  | "rules"
  | "archived";

type SettingsPageProps = {
  initialTab?: SettingsTab;
  initialRoleName?: string;
  uiTheme: Accessor<UiTheme>;
  setUiTheme: (theme: UiTheme) => void;
  assistants: Accessor<AssistantRuntime[]>;
  roles: Accessor<Role[]>;
  skills: Accessor<AppSkill[]>;
  activeSessions: AppSession[];
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  refreshSkills: () => Promise<void>;
  refreshRoles: () => Promise<void>;
  fetchRoleConfig: (runtimeKey: string, roleName?: string) => Promise<{ options: AcpConfigOption[]; modes: string[] }>;
  pushMessage: (role: string, text: string) => void;
  onRestoreSession: (id: string, title: string, activeRole: string, runtimeKind: string | null, cwd: string | null) => void;
  onBack: () => void;
};

const NAV: Array<{ id: SettingsTab; label: string; icon: typeof Settings }> = [
  { id: "general", label: "General", icon: Settings },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "configuration", label: "Configuration", icon: ShieldCheck },
  { id: "personalization", label: "Personalization", icon: Sparkles },
  { id: "mcp", label: "MCP servers", icon: Boxes },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "environments", label: "Environments", icon: Monitor },
  { id: "worktrees", label: "Worktrees", icon: BriefcaseBusiness },
  { id: "automations", label: "Automations", icon: Workflow },
  { id: "roles", label: "Roles", icon: Bot },
  { id: "rules", label: "Rules", icon: SlidersHorizontal },
  { id: "archived", label: "Archived sessions", icon: FolderArchive },
];

function ToggleRow(props: { title: string; description: string; enabled?: boolean }) {
  return (
    <div class="settings-toggle-row jui-row-button">
      <div class="min-w-0">
        <div class="text-[14px] font-medium theme-text">{props.title}</div>
        <p class="mt-1 max-w-[720px] text-[13px] leading-relaxed theme-muted">{props.description}</p>
      </div>
      <UiSwitch checked={props.enabled ?? true} />
    </div>
  );
}

export default function SettingsPage(props: SettingsPageProps) {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>(props.initialTab ?? "general");
  const [initialRoleName, setInitialRoleName] = createSignal<string | undefined>(props.initialRoleName);

  createEffect(() => {
    if (props.initialTab) setActiveTab(props.initialTab);
    setInitialRoleName(props.initialRoleName);
  });

  return (
    <div class="settings-page h-dvh overflow-hidden theme-bg theme-text">
      <aside data-tauri-drag-region class="settings-nav">
        <div class="h-[54px] shrink-0" />
        <RowButton class="settings-back" onClick={props.onBack}>
          <ArrowLeft size={17} />
          <span>Back to app</span>
        </RowButton>
        <nav class="mt-4 space-y-1 px-3">
          <For each={NAV}>
            {(item) => {
              const Icon = item.icon;
              return (
                <RowButton
                  class="settings-nav-row"
                  active={activeTab() === item.id}
                  onClick={() => setActiveTab(item.id)}
                >
                  <Icon size={15} stroke-width={1.8} />
                  <span>{item.label}</span>
                </RowButton>
              );
            }}
          </For>
        </nav>
      </aside>

      <main class="settings-main">
        <div class="settings-content">
          <Show when={activeTab() === "general"}>
            <section>
              <h1 class="settings-title">General</h1>
              <div class="settings-section">
                <h2 class="settings-section-heading">Work mode</h2>
                <div class="settings-workmode-grid">
                  <button class="settings-workmode-card is-active">
                    <div class="settings-workmode-preview">
                      <div class="mini-sidebar" />
                      <div class="mini-lines">
                        <span />
                        <span />
                        <span />
                      </div>
                      <div class="mini-output">
                        <span class="green" />
                        <span class="green short" />
                        <span class="red" />
                      </div>
                    </div>
                    <strong>For coding</strong>
                    <span>More technical responses and control</span>
                    <CheckCircle2 size={22} />
                  </button>
                  <button class="settings-workmode-card">
                    <div class="settings-workmode-preview muted">
                      <div class="mini-sidebar" />
                      <div class="mini-lines">
                        <span />
                        <span />
                        <span />
                      </div>
                    </div>
                    <strong>For everyday work</strong>
                    <span>Same power, less technical detail</span>
                    <span class="settings-radio" />
                  </button>
                </div>
              </div>
              <div class="settings-section">
                <h2 class="settings-section-heading">Permissions</h2>
                <Panel class="settings-card-list">
                  <PanelBody class="settings-card-list-body">
                  <ToggleRow title="Default permissions" description="Jockey can read and edit files in its workspace. It can ask for additional access when needed." />
                  <ToggleRow title="Auto-review" description="Jockey can automatically review requests for additional access before asking for approval." />
                  <ToggleRow title="Full access" description="When enabled, agents may run with broader filesystem and network permissions after approval." />
                  </PanelBody>
                </Panel>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "appearance"}>
            <section>
              <h1 class="settings-title">Appearance</h1>
              <div class="settings-section">
                <h2 class="settings-section-heading">Theme</h2>
                <div class="settings-theme-grid">
                  <For each={UI_THEMES}>
                    {(theme) => (
                      <button
                        type="button"
                        class="settings-theme-card"
                        classList={{ "is-active": props.uiTheme() === theme.key }}
                        onClick={() => props.setUiTheme(theme.key)}
                      >
                        <span class="theme-swatch" style={{ "background-color": theme.swatch }}>
                          <span style={{ "background-color": theme.accent }} />
                        </span>
                        <strong>{theme.label}</strong>
                        <span>{theme.description}</span>
                      </button>
                    )}
                  </For>
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "configuration"}>
            <section>
              <h1 class="settings-title">Configuration</h1>
              <div class="settings-section">
                <h2 class="settings-section-heading">Assistant runtime</h2>
                <Panel class="settings-card-list">
                  <PanelBody class="settings-card-list-body">
                  <For each={props.assistants()}>
                    {(assistant) => (
                      <RowButton
                        disabled={!assistant.available}
                        class="settings-runtime-row"
                        active={props.activeSession()?.runtimeKind === assistant.key}
                        onClick={() => assistant.available && props.patchActiveSession({ runtimeKind: assistant.key })}
                      >
                        <span class={`settings-runtime-dot ${assistant.available ? "is-online" : "is-offline"}`} />
                        <div class="min-w-0 flex-1 text-left">
                          <div class="text-[14px] font-medium theme-text">{assistant.label}</div>
                          <div class="mt-1 truncate text-[12px] theme-muted">{assistant.available ? assistant.version ?? assistant.key : assistant.installHint ?? "Unavailable"}</div>
                        </div>
                        <Show when={props.activeSession()?.runtimeKind === assistant.key}>
                          <CheckCircle2 size={18} class="theme-accent" />
                        </Show>
                      </RowButton>
                    )}
                  </For>
                  </PanelBody>
                </Panel>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "personalization"}>
            <section>
              <h1 class="settings-title">Personalization</h1>
              <div class="settings-section">
                <h2 class="settings-section-heading">Skills</h2>
                <div class="settings-management-panel">
                  <SkillRegistryTab skills={props.skills()} refreshSkills={props.refreshSkills} />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "mcp"}>
            <section>
              <h1 class="settings-title">MCP servers</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <McpRegistryTab pushMessage={props.pushMessage} />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "git"}>
            <section>
              <h1 class="settings-title">Git</h1>
              <div class="settings-section">
                <h2 class="settings-section-heading">Repository</h2>
                <Panel class="settings-card-list">
                  <PanelBody class="settings-card-list-body">
                  <ToggleRow title="Show branch state" description="Display branch and dirty state in the workspace toolbar." />
                  <ToggleRow title="Open diffs in the main work area" description="Keep file and diff preview wide above the conversation." />
                  </PanelBody>
                </Panel>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "environments"}>
            <section>
              <h1 class="settings-title">Environments</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <ExternalAgentsTab />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "worktrees"}>
            <section>
              <h1 class="settings-title">Worktrees</h1>
              <div class="settings-section">
                <div class="settings-empty">
                  <Code2 size={22} />
                  <span>Worktree settings will be added when worktree management is wired.</span>
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "automations"}>
            <section>
              <h1 class="settings-title">Automations</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <WorkflowsTab roles={props.roles()} />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "roles"}>
            <section>
              <h1 class="settings-title">Roles</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <RolesTab
                    roles={props.roles}
                    activeSession={props.activeSession}
                    patchActiveSession={props.patchActiveSession}
                    updateSession={props.updateSession}
                    refreshRoles={props.refreshRoles}
                    fetchRoleConfig={props.fetchRoleConfig}
                    pushMessage={props.pushMessage}
                    initialRoleName={initialRoleName()}
                  />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "rules"}>
            <section>
              <h1 class="settings-title">Rules</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <RulesTab />
                </div>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "archived"}>
            <section>
              <h1 class="settings-title">Archived sessions</h1>
              <div class="settings-section">
                <div class="settings-management-panel">
                  <SessionsTab activeSessions={props.activeSessions} onRestoreSession={props.onRestoreSession} />
                </div>
              </div>
            </section>
          </Show>
        </div>
      </main>
    </div>
  );
}
