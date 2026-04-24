import { createSignal } from "solid-js";
import type { Role, AssistantRuntime, AppSkill } from "../components/types";
import { DEFAULT_ROLE_ALIAS, DEFAULT_BACKEND_ROLE } from "../components/types";
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
    if (k === "claude" || k === "claude-acp") return "claude-code";
    if (k === "gemini") return "gemini-cli";
    if (k === "codex" || k === "codex-acp") return "codex-cli";
    return k;
  };

  const isCustomRole = () => {
    const s = activeSession();
    return s ? s.activeRole !== DEFAULT_ROLE_ALIAS && s.activeRole !== DEFAULT_BACKEND_ROLE : false;
  };

  const activeBackendRole = () => isCustomRole() ? (activeSession()?.activeRole ?? DEFAULT_BACKEND_ROLE) : DEFAULT_BACKEND_ROLE;

  const refreshRoles = async () => {
    try {
      const rows = await roleApi.list();
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
    patchActiveSession({ runtimeKind: assistantKey });
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
