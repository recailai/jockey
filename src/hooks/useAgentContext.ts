import { createSignal } from "solid-js";
import type { Role, AssistantRuntime, AppSkill, AcpConfigOption } from "../components/types";
import { DEFAULT_ROLE_ALIAS, DEFAULT_BACKEND_ROLE } from "../components/types";
import type { SessionManager } from "./useSessionManager";
import type { StreamEngine } from "./useStreamEngine";
import { assistantApi, roleApi, skillApi } from "../lib/tauriApi";
import { createRunTokenSource, type RunToken } from "../lib/runToken";

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
    patchActiveSession,
    pushMessage,
    getSessionIndex,
  } = sessionManager;

  const { acceptingStreams, finalizeSessionStream } = streamEngine;

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

  const runTokens = createRunTokenSource();

  const getRunToken = () => runTokens.current();
  const bumpRunToken = () => runTokens.next();
  const getCanceledRunToken = () => runTokens.cancelledUpTo();
  const isRunCancelled = (token: RunToken) => runTokens.isCancelled(token);

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
    const cidx = sid ? getSessionIndex(sid) : -1;
    const sess = cidx !== -1 ? sessions[cidx] : null;
    if (!sess?.submitting || !sid) return;

    const role = activeBackendRole();

    // Preserve any partial output the user has already seen: flush
    // streamingMessage (plus tool calls / segments) into a persistent message
    // before we mark this run cancelled. Without this, mid-stream cancel erases
    // everything that was rendered live.
    //
    // expectedRunToken is the current (pre-cancel) token, so finalize targets
    // the right run; finalize itself clears streamingMessage + streamingRunToken.
    // Only finalize when we actually have an owning run — otherwise the stream
    // isn't ours to finalize.
    if (sess.streamingRunToken !== null && sess.streamingMessage) {
      finalizeSessionStream(sid, role, undefined, sess.streamingRunToken);
    }

    // Mark current run token as cancelled so its pending sendRaw promise bails
    // out of post-await work (see useMessageSend).
    runTokens.markCancelled();
    acceptingStreams.delete(sid);
    updateSession(sid, {
      currentPlan: null,
      pendingPermission: null,
      submitting: false,
      status: "idle",
    });
    pushMessage("event", "Cancellation requested.");

    // Await cancelSession — the Rust worker now blocks on the old prompt's
    // PROMPT_LOCK (up to 5s) before responding, so by the time this resolves
    // the old turn has drained and we can safely launch the next queued send.
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
    isRunCancelled,
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
