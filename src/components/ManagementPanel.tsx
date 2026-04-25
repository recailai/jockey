/**
 * ManagementPanel — full-screen management hub
 * Tabs: Sessions · Workflows · MCP Registry · Skill Registry
 *
 * Aesthetic: terminal-grid brutalism — tight monospace data density,
 * hairline dividers, amber/teal/indigo accent pills, zero decorative chrome.
 * The panel slides in from the right over a dimmed backdrop.
 */

import { For, Show, createEffect, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession, AppSkill, Role, AcpConfigOption } from "./types";
import { INTERACTIVE_MOTION } from "./types";
import { TABS, type TabId } from "./management/primitives";
import { SessionsTab } from "./management/SessionsTab";
import { WorkflowsTab } from "./management/WorkflowsTab";
import { RolesTab } from "./management/RolesTab";
import { McpRegistryTab } from "./management/McpRegistryTab";
import { SkillRegistryTab } from "./management/SkillRegistryTab";
import { RulesTab } from "./management/RulesTab";

// ─────────────────────────────────────────────────────────────────────────────
// Root ManagementPanel
// ─────────────────────────────────────────────────────────────────────────────

export type ManagementPanelProps = {
  show: Accessor<boolean>;
  onClose: () => void;
  initialTab?: TabId;
  initialRoleName?: string;
  activeSessions: AppSession[];
  onRestoreSession?: (id: string, title: string, activeRole: string, runtimeKind: string | null, cwd: string | null) => void;
  skills: Accessor<AppSkill[]>;
  roles: Accessor<Role[]>;
  refreshSkills: () => Promise<void>;
  // Roles tab extras
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
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
    const target = e.target;
    if (!(target instanceof Node)) return;
    if (!(e.currentTarget as HTMLElement).contains(target)) return;
    if (target instanceof Element && target.closest("[data-panel]")) return;
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
          class="ml-auto flex h-full w-[760px] max-w-[92vw] flex-col shadow-2xl shadow-black/60 theme-surface"
          style="animation: slideInRight 180ms cubic-bezier(0.16,1,0.3,1) both"
        >
          {/* Top bar */}
          <div class="flex h-9 shrink-0 items-center backdrop-blur-md theme-surface-muted">
            {/* Nav tabs */}
            <div class="flex items-stretch h-full">
              <For each={TABS}>
                {(tab) => (
                  <button
                    onClick={() => setActiveTab(tab.id)}
                    class={`flex h-full items-center gap-1.5 border-b-[1.5px] px-4 font-mono text-[10px] font-semibold uppercase tracking-widest transition-colors duration-150 ${
                      activeTab() === tab.id
                        ? "border-[var(--ui-text)] theme-text"
                        : "border-transparent theme-muted hover:border-[var(--ui-border-strong)] hover:theme-text"
                    }`}
                  >
                    <span class={activeTab() === tab.id ? "theme-text" : "theme-muted"}>
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
              class={`mr-3 flex h-6 w-6 items-center justify-center rounded-md theme-muted hover:bg-[var(--ui-accent-soft)] hover:text-primary ${INTERACTIVE_MOTION}`}
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
              <SessionsTab activeSessions={props.activeSessions} onRestoreSession={props.onRestoreSession} />
            </Show>
            <Show when={activeTab() === "workflows"}>
              <WorkflowsTab roles={props.roles()} />
            </Show>
            <Show when={activeTab() === "roles"}>
              <RolesTab
                roles={props.roles}
                activeSession={props.activeSession}
                patchActiveSession={props.patchActiveSession}
                updateSession={props.updateSession}
                refreshRoles={props.refreshRoles}
                fetchConfigOptions={props.fetchConfigOptions}
                pushMessage={props.pushMessage}
                initialRoleName={props.initialRoleName}
              />
            </Show>
            <Show when={activeTab() === "mcp"}>
              <McpRegistryTab pushMessage={props.pushMessage} />
            </Show>
            <Show when={activeTab() === "skills"}>
              <SkillRegistryTab skills={props.skills()} refreshSkills={props.refreshSkills} />
            </Show>
            <Show when={activeTab() === "rules"}>
              <RulesTab />
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
