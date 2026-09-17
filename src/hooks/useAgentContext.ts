import type { AcpConfigOption } from "../components/types";
import type { SessionManager } from "./useSessionManager";
import type { StreamEngine } from "./useStreamEngine";
import { assistantApi } from "../lib/tauriApi";
import { useAgentData } from "./useAgentData";
import { useRunTokens } from "./useRunTokens";

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
    mutateSession,
    pushMessage,
    getSessionIndex,
  } = sessionManager;

  const { acceptingStreams, finalizeSessionStream } = streamEngine;

  const agentData = useAgentData(sessionManager, showToast);
  const {
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
  } = agentData;

  const runTokens = useRunTokens();
  const { getRunToken, bumpRunToken, getCanceledRunToken, isRunCancelled } = runTokens;

  const commandCacheKey = (runtimeKey: string, roleName: string) => `${runtimeKey}:${roleName}`;

  const runtimeForRole = (roleName: string): string | null => {
    return roles().find((r) => r.roleName === roleName)?.runtimeKind ?? activeSession()?.runtimeKind ?? null;
  };

  /** The catalog for one (persona, engine). Deliberately not memoized here: the backend owns
   *  that cache, keyed by runtime and stamped with the shape and age it was derived from, so a
   *  second copy on this side could only disagree with it. This also no longer writes session
   *  state as a side effect — App.tsx's effect is the single owner of what gets rendered. */
  const fetchRoleConfig = async (
    runtimeKey: string,
    roleName?: string,
    forceRefresh = false,
  ): Promise<{ options: AcpConfigOption[]; modes: string[] }> => {
    const empty = { options: [] as AcpConfigOption[], modes: [] as string[] };
    try {
      const resolvedRole = roleName ?? normalizeRuntimeKey(runtimeKey);
      // Last-resort shape for a runtime that reports nothing: whatever discovery last
      // persisted onto the persona row.
      const roleStoredOptions = (): AcpConfigOption[] => {
        const role = roles().find((r) => r.roleName === resolvedRole);
        if (!role) return [];
        try {
          const parsed = JSON.parse(role.configOptionDefsJson || "[]");
          return Array.isArray(parsed) ? (parsed as AcpConfigOption[]) : [];
        } catch { return []; }
      };
      const sid = activeSessionId();
      if (!sid) return { options: roleStoredOptions(), modes: [] };
      const projectId = activeSession()?.projectId ?? undefined;

      const result = await assistantApi.prewarmRoleConfig(resolvedRole, sid, projectId, forceRefresh);
      const opts = result.configOptions as AcpConfigOption[];
      const modes = result.modes as string[];
      if (opts.length > 0) void refreshRoles(projectId);
      return { options: opts.length > 0 ? opts : roleStoredOptions(), modes };
    } catch {
      return empty;
    }
  };

  const fetchModes = async (runtimeKey: string, roleName?: string): Promise<string[]> => {
    const { modes } = await fetchRoleConfig(runtimeKey, roleName);
    return modes;
  };

  const fetchConfigOptions = async (runtimeKey: string, roleName?: string): Promise<AcpConfigOption[]> => {
    const { options } = await fetchRoleConfig(runtimeKey, roleName);
    return options;
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
      const raw = await assistantApi.listAvailableCommands(roleName, sid, activeSession()?.projectId);
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

    const aidx = getSessionIndex(sessionId);
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
      mutateSession(sid, (s) => {
        const next = new Map(s.agentCommands);
        for (const key of next.keys()) {
          if (key.endsWith(`:${role}`)) next.delete(key);
        }
        s.agentCommands = next;
      });
      updateSession(sid, {
        discoveredConfigOptions: [],
        toolCalls: {},
        streamSegments: [],
        currentPlan: null,
      usage: null,
      notices: [],
      pendingUserInput: [],
        pendingPermissions: [],
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
      mutateSession(sid, (s) => {
        const next = new Map(s.agentCommands);
        for (const key of next.keys()) {
          if (key.endsWith(`:${role}`)) next.delete(key);
        }
        s.agentCommands = next;
      });
      updateSession(sid, {
        discoveredConfigOptions: [],
        toolCalls: {},
        streamSegments: [],
        currentPlan: null,
      usage: null,
      notices: [],
      pendingUserInput: [],
        pendingPermissions: [],
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

  const cancelCurrentRun = async (runNextQueued: () => void, clearSessionStream?: (sid: string) => void) => {
    const sid = activeSessionId();
    const cidx = sid ? getSessionIndex(sid) : -1;
    const sess = cidx !== -1 ? sessions[cidx] : null;
    if (!sess?.submitting || !sid) return;

    const role = activeBackendRole();

    if (sess.streamingRunToken !== null && sess.streamingMessage) {
      finalizeSessionStream(sid, role, undefined, sess.streamingRunToken);
    }

    runTokens.markCancelled();
    acceptingStreams.delete(sid);
    clearSessionStream?.(sid);
    updateSession(sid, {
      currentPlan: null,
      usage: null,
      notices: [],
      pendingUserInput: [],
      pendingPermissions: [],
      submitting: false,
      status: "idle",
    });
    pushMessage("event", "Cancellation requested.");

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
    normalizeRuntimeKey,
    commandCacheKey,
    getRunToken,
    bumpRunToken,
    getCanceledRunToken,
    isRunCancelled,
    isCustomRole,
    activeBackendRole,
    runtimeForRole,
    refreshRoles,
    refreshSkills,
    fetchRoleConfig,
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
