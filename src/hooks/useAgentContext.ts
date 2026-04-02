import { createSignal } from "solid-js";
import { produce } from "solid-js/store";
import type { Role, AssistantRuntime, AppSkill, AcpConfigOption } from "../components/types";
import { DEFAULT_ROLE_ALIAS, DEFAULT_BACKEND_ROLE } from "../components/types";
import type { SessionManager } from "./useSessionManager";
import type { StreamEngine } from "./useStreamEngine";
import { assistantApi, roleApi, skillApi } from "../lib/tauriApi";

export function useAgentContext(
  sessionManager: SessionManager,
  streamEngine: StreamEngine,
  showToast: (message: string, severity?: "error" | "info") => void,
) {
  const {
    sessions,
    setSessions,
    activeSessionId,
    activeSession,
    updateSession,
    patchActiveSession,
    pushMessage,
  } = sessionManager;

  const { finalizeSessionStream, acceptingStreams } = streamEngine;

  const [roles, setRoles] = createSignal<Role[]>([]);
  const [assistants, setAssistants] = createSignal<AssistantRuntime[]>([]);
  const [skills, setSkills] = createSignal<AppSkill[]>([]);

  const runtimeConfigCache = new Map<string, { options: AcpConfigOption[]; modes: string[] }>();

  const normalizeRuntimeKey = (runtimeKey: string): string => {
    const k = runtimeKey.trim().toLowerCase();
    if (k === "claude" || k === "claude-acp") return "claude-code";
    if (k === "gemini") return "gemini-cli";
    if (k === "codex" || k === "codex-acp") return "codex-cli";
    return k;
  };
  const commandCacheKey = (runtimeKey: string, roleName: string) => `${runtimeKey}:${roleName}`;

  let runTokenSeq = 0;
  let canceledRunToken = 0;

  const getRunToken = () => runTokenSeq;
  const bumpRunToken = () => ++runTokenSeq;
  const getCanceledRunToken = () => canceledRunToken;
  const setCanceledRunToken = (v: number) => { canceledRunToken = v; };

  const isCustomRole = () => {
    const s = activeSession();
    return s ? s.activeRole !== DEFAULT_ROLE_ALIAS && s.activeRole !== DEFAULT_BACKEND_ROLE : false;
  };

  const activeBackendRole = () => isCustomRole() ? (activeSession()?.activeRole ?? DEFAULT_BACKEND_ROLE) : DEFAULT_BACKEND_ROLE;

  const runtimeForRole = (roleName: string): string | null => {
    if (!isCustomRole() || roleName === DEFAULT_BACKEND_ROLE) return activeSession()?.runtimeKind ?? null;
    return roles().find((r) => r.roleName === roleName)?.runtimeKind ?? null;
  };

  // slashCliCache is shared between useAgentContext and useCompletions;
  // we keep it here and expose a reset helper used by refreshRoles/refreshAssistants
  let slashCliCacheRef: { cache: null | unknown[]; version: number } = { cache: null, version: 0 };

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

  const fetchConfigOptions = async (runtimeKey: string, roleName?: string): Promise<AcpConfigOption[]> => {
    try {
      const resolvedRole = roleName ?? normalizeRuntimeKey(runtimeKey);
      const roleStoredOptions = (): AcpConfigOption[] => {
        const role = roles().find((r) => r.roleName === resolvedRole);
        if (!role) return [];
        try {
          const parsed = JSON.parse(role.configOptionDefsJson || "[]");
          return Array.isArray(parsed) ? (parsed as AcpConfigOption[]) : [];
        } catch {
          return [];
        }
      };
      const sid = activeSessionId();
      if (!sid) return roleStoredOptions();
      const hit = runtimeConfigCache.get(resolvedRole);
      if (hit) {
        void assistantApi.prewarmRoleConfig(resolvedRole, sid).catch(() => {});
        return hit.options;
      }
      const cached = await assistantApi.listDiscoveredConfig(resolvedRole);
      if ((cached as AcpConfigOption[]).length > 0) {
        runtimeConfigCache.set(resolvedRole, { options: cached as AcpConfigOption[], modes: [] });
        void assistantApi.prewarmRoleConfig(resolvedRole, sid).catch(() => {});
        return cached as AcpConfigOption[];
      }
      const result = await assistantApi.prewarmRoleConfig(resolvedRole, sid);
      const opts = result.configOptions as AcpConfigOption[];
      runtimeConfigCache.set(resolvedRole, { options: opts, modes: result.modes });
      return opts.length > 0 ? opts : roleStoredOptions();
    } catch {
      const resolvedRole = roleName ?? normalizeRuntimeKey(runtimeKey);
      const role = roles().find((r) => r.roleName === resolvedRole);
      if (!role) return [];
      try {
        const parsed = JSON.parse(role.configOptionDefsJson || "[]");
        return Array.isArray(parsed) ? (parsed as AcpConfigOption[]) : [];
      } catch {
        return [];
      }
    }
  };

  const fetchModes = async (runtimeKey: string, roleName?: string): Promise<string[]> => {
    try {
      const resolvedRole = roleName ?? normalizeRuntimeKey(runtimeKey);
      const hit = runtimeConfigCache.get(resolvedRole);
      if (hit?.modes.length) return hit.modes;
      return await assistantApi.listDiscoveredModes(resolvedRole);
    } catch { return []; }
  };

  const parseAgentCommands = (raw: unknown[]): Array<{ name: string; description: string; hint?: string }> => {
    return (raw as Array<{ name: string; description?: string; input?: { hint?: string } }>).map((c) => ({
      name: c.name, description: c.description ?? "", hint: c.input?.hint,
    }));
  };

  const fetchAgentCommands = async (runtimeKey: string, roleName: string): Promise<{ runtimeKey: string; commands: Array<{ name: string; description: string; hint?: string }> }> => {
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    try {
      const sid = activeSessionId();
      if (!sid) return { runtimeKey: normalizedRuntime, commands: [] };
      const raw = await assistantApi.listAvailableCommands(roleName, sid);
      return { runtimeKey: normalizedRuntime, commands: parseAgentCommands(raw) };
    } catch {
      return { runtimeKey: normalizedRuntime, commands: [] };
    }
  };

  const hydrateAgentCommandsForSession = async (
    sessionId: string,
    runtimeKey: string,
    roleName: string,
  ): Promise<number> => {
    const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
    const result = await fetchAgentCommands(normalizedRuntime, roleName);

    const aidx = sessions.findIndex((sess) => sess.id === sessionId);
    const commandKey = commandCacheKey(result.runtimeKey, roleName);
    if (aidx !== -1) {
      setSessions(aidx, "agentCommands", (m) => {
        const next = new Map(m);
        next.set(commandKey, result.commands);
        return next;
      });
    }
    return result.commands.length;
  };

  const fetchAndCacheAgentCommands = (runtimeKey: string, roleName: string) => {
    void (async () => {
      const sid = activeSessionId();
      if (!sid) return;
      await hydrateAgentCommandsForSession(sid, runtimeKey, roleName);
    })().catch((e: unknown) => {
      showToast(`Commands unavailable for ${roleName}: ${String(e)}`, "info");
    });
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

  const resetActiveAgentContext = async () => {
    const sid = activeSessionId();
    const role = activeBackendRole();
    if (!sid) {
      showToast("No active agent context to reset.", "info");
      return;
    }
    if (activeSession()?.submitting) {
      showToast("Stop current run before resetting context.", "info");
      return;
    }
    try {
      await assistantApi.resetSession(role, sid);
      setSessions((s) => s.id === sid, produce((s) => {
        const next = new Map(s.agentCommands);
        for (const key of next.keys()) {
          if (key.endsWith(`:${role}`)) next.delete(key);
        }
        s.agentCommands = next;
      }));
      updateSession(sid, {
        discoveredConfigOptions: [],
        toolCalls: {},
        streamSegments: [],
        currentPlan: null,
        pendingPermission: null,
        thoughtText: "",
        agentState: undefined,
        currentMode: null,
        agentModes: [],
      });
      pushMessage("event", `[${role}] CLI context reset.`);
    } catch (e) {
      showToast(`Failed to reset ${role} context: ${String(e)}`);
    }
  };

  const reconnectActiveAgent = async () => {
    const sid = activeSessionId();
    const role = activeBackendRole();
    if (!sid) {
      showToast("No active session to reconnect.", "info");
      return;
    }
    if (activeSession()?.submitting) {
      showToast("Stop current run before reconnecting.", "info");
      return;
    }
    try {
      await assistantApi.reconnectSession(role, sid);
      setSessions((s) => s.id === sid, produce((s) => {
        const next = new Map(s.agentCommands);
        for (const key of next.keys()) {
          if (key.endsWith(`:${role}`)) next.delete(key);
        }
        s.agentCommands = next;
      }));
      updateSession(sid, {
        discoveredConfigOptions: [],
        toolCalls: {},
        streamSegments: [],
        currentPlan: null,
        pendingPermission: null,
        thoughtText: "",
        agentState: undefined,
        currentMode: null,
        agentModes: [],
      });
      pushMessage("event", `[${role}] Reconnected — MCP changes will apply on next message.`);
    } catch (e) {
      showToast(`Failed to reconnect ${role}: ${String(e)}`);
    }
  };

  const cancelCurrentRun = async (runNextQueued: () => void) => {
    const sid = activeSessionId();
    const sess = sid ? sessions.find((s) => s.id === sid) : null;
    if (!sess?.submitting || !sid) return;
    canceledRunToken = Math.max(canceledRunToken, runTokenSeq);
    acceptingStreams.delete(sid);
    finalizeSessionStream(sid, activeBackendRole());
    updateSession(sid, { toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, thoughtText: "", submitting: false, status: "idle" });
    pushMessage("event", "Cancellation requested.");
    const role = activeBackendRole();
    try {
      await assistantApi.cancelSession(role, sid);
    } catch { }
    runNextQueued();
  };

  return {
    roles,
    setRoles,
    assistants,
    setAssistants,
    skills,
    setSkills,
    runtimeConfigCache,
    normalizeRuntimeKey,
    commandCacheKey,
    getRunToken,
    bumpRunToken,
    getCanceledRunToken,
    setCanceledRunToken,
    isCustomRole,
    activeBackendRole,
    runtimeForRole,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
    fetchModes,
    parseAgentCommands,
    hydrateAgentCommandsForSession,
    fetchAndCacheAgentCommands,
    setPreferredAssistant,
    refreshAssistants,
    resetActiveAgentContext,
    reconnectActiveAgent,
    cancelCurrentRun,
    slashCliCacheRef,
  };
}

export type AgentContext = ReturnType<typeof useAgentContext>;
