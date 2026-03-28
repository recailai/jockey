import { createSignal } from "solid-js";
import { produce } from "solid-js/store";
import { invoke } from "@tauri-apps/api/core";
import type { Role, AssistantRuntime, AppSkill, AcpConfigOption } from "../components/types";
import { DEFAULT_ROLE_ALIAS, DEFAULT_BACKEND_ROLE } from "../components/types";
import type { SessionManager } from "./useSessionManager";
import type { StreamEngine } from "./useStreamEngine";

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

  const runtimeConfigCache = new Map<string, AcpConfigOption[]>();

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
      const rows = await invoke<Role[]>("list_roles");
      setRoles(rows);
      slashCliCacheRef.cache = null;
    } catch { setRoles([]); }
  };

  const refreshSkills = async () => {
    try {
      const rows = await invoke<AppSkill[]>("list_app_skills");
      setSkills(rows);
    } catch { setSkills([]); }
  };

  const fetchConfigOptions = async (runtimeKey: string, roleName?: string): Promise<AcpConfigOption[]> => {
    try {
      const sid = activeSessionId();
      if (!sid) return [];
      const normalizedRuntime = normalizeRuntimeKey(runtimeKey);
      if (roleName) {
        const raw = await invoke<unknown[]>("prewarm_role_config_cmd", {
          runtimeKind: normalizedRuntime,
          roleName,
          appSessionId: sid,
        });
        return raw as AcpConfigOption[];
      }
      const hit = runtimeConfigCache.get(normalizedRuntime);
      if (hit) return hit;
      const raw = await invoke<unknown[]>("prewarm_role_config_cmd", {
        runtimeKind: normalizedRuntime,
        roleName: "UnionAIAssistant",
        appSessionId: sid,
      });
      const opts = raw as AcpConfigOption[];
      if (opts.length > 0) runtimeConfigCache.set(normalizedRuntime, opts);
      return opts;
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
      const raw = await invoke<unknown[]>("list_available_commands_cmd", {
        runtimeKey: normalizedRuntime,
        roleName,
        appSessionId: sid,
      });
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
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
    slashCliCacheRef.cache = null;
    const current = activeSession()?.runtimeKind ?? null;
    const currentAvailable = current ? rows.find((a) => a.key === current && a.available) : null;
    if (currentAvailable) return;
    const first = rows.find((a) => a.available) ?? null;
    if (first) {
      setPreferredAssistant(first.key);
    }
  };

  const resetActiveAgentContext = async () => {
    const sid = activeSessionId();
    const role = activeBackendRole();
    const runtime = runtimeForRole(role);
    if (!sid || !runtime) {
      showToast("No active agent context to reset.", "info");
      return;
    }
    if (activeSession()?.submitting) {
      showToast("Stop current run before resetting context.", "info");
      return;
    }
    try {
      await invoke("reset_acp_session", { runtimeKind: runtime, roleName: role, appSessionId: sid });
      const cacheKey = commandCacheKey(normalizeRuntimeKey(runtime), role);
      setSessions((s) => s.id === sid, produce((s) => {
        const next = new Map(s.agentCommands);
        next.delete(cacheKey);
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
    const runtime = runtimeForRole(role);
    if (runtime) {
      try {
        await invoke("cancel_acp_session", { runtimeKind: runtime, roleName: role, appSessionId: sid });
      } catch { }
    }
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
    parseAgentCommands,
    hydrateAgentCommandsForSession,
    fetchAndCacheAgentCommands,
    setPreferredAssistant,
    refreshAssistants,
    resetActiveAgentContext,
    cancelCurrentRun,
    slashCliCacheRef,
  };
}

export type AgentContext = ReturnType<typeof useAgentContext>;
