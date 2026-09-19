import { createSignal } from "solid-js";
import type { Role, AssistantRuntime, AppSkill } from "../components/types";
import { DEFAULT_BACKEND_ROLE } from "../components/types";
import type { SessionManager } from "./useSessionManager";
import { assistantApi, roleApi, skillApi } from "../lib/tauriApi";

export function useAgentData(
  sessionManager: SessionManager,
  showToast: (message: string, severity?: "error" | "info") => void,
) {
  const { activeSession, patchActiveSession } = sessionManager;

  const [roles, setRoles] = createSignal<Role[]>([]);
  const [assistants, setAssistants] = createSignal<AssistantRuntime[]>([]);
  const [skills, setSkills] = createSignal<AppSkill[]>([]);

  const slashCliCacheRef: { cache: null | unknown[]; version: number } = { cache: null, version: 0 };

  const normalizeRuntimeKey = (runtimeKey: string): string => {
    const k = runtimeKey.trim().toLowerCase();
    if (k === "claude") return "claude-native";
    if (k === "claude-acp") return "claude-code";
    if (k === "agy" || k === "antigravity" || k === "gemini" || k === "gemini-cli") return "antigravity-cli";
    if (k === "codex" || k === "codex-acp") return "codex-cli";
    if (k === "pi" || k === "pi-coding-agent") return "pi-cli";
    return k;
  };

  const isCustomRole = () => true;

  const activeBackendRole = () => activeSession()?.activeRole ?? DEFAULT_BACKEND_ROLE;

  const refreshRoles = async (projectId?: string) => {
    try {
      const rows = await roleApi.list(projectId);
      setRoles(rows);
      slashCliCacheRef.cache = null;
    } catch (e) {
      showToast(`Failed to load roles: ${String(e)}`);
    }
  };

  const refreshSkills = async () => {
    try {
      const rows = await skillApi.list();
      setSkills(rows);
    } catch (e) {
      showToast(`Failed to load skills: ${String(e)}`);
    }
  };

  const setPreferredAssistant = (assistantKey: string | null) => {
    const profile = assistantKey ? assistants().find((assistant) => assistant.key === assistantKey) : null;
    patchActiveSession({
      runtimeKind: assistantKey,
      runtimeProfileId: profile?.profileId ?? null,
    });
  };

  const refreshAssistants = async () => {
    try {
      const rows = await assistantApi.detect();
      setAssistants(rows);
      slashCliCacheRef.cache = null;
      const current = activeSession()?.runtimeKind ?? null;
      const currentAvailable = current ? rows.find((a) => a.key === current && a.available) : null;
      if (currentAvailable) return;
      const first = rows.find((a) => a.available) ?? null;
      if (first) {
        setPreferredAssistant(first.key);
      }
    } catch (e) {
      showToast(`Failed to detect assistants: ${String(e)}`);
    }
  };

  return {
    roles,
    setRoles,
    assistants,
    setAssistants,
    skills,
    setSkills,
    slashCliCacheRef,
    normalizeRuntimeKey,
    isCustomRole,
    activeBackendRole,
    refreshRoles,
    refreshSkills,
    setPreferredAssistant,
    refreshAssistants,
  };
}

export type AgentData = ReturnType<typeof useAgentData>;
