import type { AppMessage, AppSession } from "../components/types";
import { now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS } from "../components/types";
import { appSessionApi, assistantApi } from "../lib/tauriApi";
import { parseAgentControlCommand, resolveRoute } from "../lib/chatPipeline";
import { isDefaultSessionTitle, deriveSessionTitleFromMessage } from "../lib/sessionHelpers";
import type { SessionManager } from "./useSessionManager";
import type { StreamEngine } from "./useStreamEngine";
import type { AgentContext } from "./useAgentContext";

type MessageSendDeps = {
  sessionManager: SessionManager;
  streamEngine: StreamEngine;
  agentContext: AgentContext;
  closeMentionMenu: () => void;
  closeSlashMenu: () => void;
  showToast: (msg: string, severity?: "error" | "info") => void;
};

export function useMessageSend({
  sessionManager,
  streamEngine,
  agentContext,
  closeMentionMenu,
  closeSlashMenu,
  showToast,
}: MessageSendDeps) {
  const {
    sessions, setSessions,
    activeSessionId, activeSession,
    updateSession, patchActiveSession,
    appendMessageToSession, pushMessage,
    scheduleScrollToBottom,
    getSessionIndex,
  } = sessionManager;

  const {
    acceptingStreams,
    appendThought: _appendThought,
    resetStreamState, finalizeSessionStream,
  } = streamEngine;

  const {
    roles,
    bumpRunToken, getCanceledRunToken,
    isCustomRole, activeBackendRole,
    refreshRoles,
    fetchConfigOptions, fetchAndCacheAgentCommands,
    setPreferredAssistant,
    cancelCurrentRun: cancelCurrentRunBase,
  } = agentContext;

  // --- helpers ---

  const queuedInputsFor = (sid: string | null): string[] => {
    if (!sid) return [];
    const idx = getSessionIndex(sid);
    return idx !== -1 ? (sessions[idx]?.queuedMessages ?? []) : [];
  };

  const patchSessionById = (sessionId: string | null, patch: Partial<AppSession>) => {
    if (!sessionId) return;
    updateSession(sessionId, patch);
  };

  const applyRouteState = (route: ReturnType<typeof resolveRoute>) => {
    if (route.activateRole === DEFAULT_ROLE_ALIAS) {
      patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS });
      return;
    }
    if (route.activateRole) {
      patchActiveSession({ activeRole: route.activateRole, discoveredConfigOptions: [] });
    }
  };

  const prefetchRoleResources = (roleName: string) => {
    const targetRole = roles().find((r) => r.roleName === roleName);
    if (!targetRole) return;
    void fetchConfigOptions(targetRole.runtimeKind, targetRole.roleName).then(
      (opts) => patchActiveSession({ discoveredConfigOptions: opts })
    );
    fetchAndCacheAgentCommands(targetRole.runtimeKind, targetRole.roleName);
  };

  const maybeAutoTitleSession = (sid: string, text: string) => {
    const sidx = getSessionIndex(sid);
    const sess = sidx !== -1 ? sessions[sidx] : null;
    if (!sess) return;
    if (!isDefaultSessionTitle(sess.title)) return;
    if (sess.messages.filter((m) => m.roleName === "user").length !== 0) return;
    const existing = sessions.filter((x) => x.id !== sid).map((x) => x.title);
    const autoTitle = deriveSessionTitleFromMessage(text, existing);
    updateSession(sid, { title: autoTitle });
    void appSessionApi.update(sid, { title: autoTitle }).catch(() => {});
  };

  const buildStreamOps = (originSessionId: string | null, sendRoleLabel: string) => {
    const appendOriginMessage = (msg: AppMessage) => {
      if (!originSessionId) return;
      appendMessageToSession(originSessionId, msg);
    };
    const startOriginStream = () => {
      if (originSessionId) acceptingStreams.add(originSessionId);
      const id = `stream-${now()}`;
      const row: AppMessage = { id, roleName: sendRoleLabel, text: "", at: now() };
      patchSessionById(originSessionId, {
        streamingMessage: row, toolCalls: {}, streamSegments: [],
        currentPlan: null, pendingPermission: null, thoughtText: "",
      });
      scheduleScrollToBottom();
      return row;
    };
    const completeOriginStream = (finalReply?: string) => {
      if (!originSessionId) return;
      finalizeSessionStream(originSessionId, sendRoleLabel, finalReply);
    };
    const dropOriginStream = () => {
      patchSessionById(originSessionId, { streamingMessage: null, thoughtText: "" });
      resetStreamState(originSessionId ?? undefined);
    };
    return { appendOriginMessage, startOriginStream, completeOriginStream, dropOriginStream };
  };

  const runAgentControlCommand = async (
    text: string,
    isCommand: boolean,
    inRoleContext: boolean,
    originSessionId: string | null,
    patchOriginSession: (patch: Partial<AppSession>) => void,
  ): Promise<boolean> => {
    const cmd = parseAgentControlCommand(text, isCommand, inRoleContext);
    if (!cmd) return false;
    const role = activeBackendRole();
    try {
      if (cmd === "cancel") {
        if (originSessionId) await assistantApi.cancelSession(role, originSessionId);
        pushMessage("event", `cancelled ${role}`);
      } else {
        if (originSessionId) await assistantApi.setMode(role, cmd, originSessionId);
        pushMessage("event", `${role} mode → ${cmd}`);
      }
    } catch (e) {
      pushMessage("event", String(e));
    } finally {
      patchOriginSession({ submitting: false, status: "idle" });
      runNextQueued(originSessionId);
    }
    return true;
  };

  // --- public API ---

  const runNextQueued = (preferredSessionId?: string | null) => {
    const sid = preferredSessionId ?? activeSessionId();
    if (!sid) return;
    const idx = getSessionIndex(sid);
    const s = idx !== -1 ? sessions[idx] : null;
    if (s?.submitting) return;
    const queue = queuedInputsFor(sid);
    if (queue.length === 0) return;
    if (idx !== -1) setSessions(idx, "queuedMessages", []);
    const merged = queue.map((q) => q.trim()).filter(Boolean).join("\n");
    if (!merged) return;
    if (queue.length > 1) {
      appendMessageToSession(sid, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: "event",
        text: `queued messages merged: ${queue.length}`,
        at: now(),
      });
    }
    void sendRaw(merged, false, sid);
  };

  const cancelCurrentRun = async () => {
    await cancelCurrentRunBase(runNextQueued);
  };

  const sendRaw = async (text: string, silent = false, targetSessionId?: string | null) => {
    const runToken = bumpRunToken();
    const originSessionId = targetSessionId ?? activeSessionId();
    closeMentionMenu();
    closeSlashMenu();

    const patchOriginSession = (patch: Partial<AppSession>) => patchSessionById(originSessionId, patch);

    const _oidx = originSessionId ? getSessionIndex(originSessionId) : -1;
    const s = (_oidx !== -1 ? sessions[_oidx] : null) ?? activeSession();
    const sessionIsCustomRole = s
      ? s.activeRole !== DEFAULT_ROLE_ALIAS && s.activeRole !== DEFAULT_BACKEND_ROLE
      : false;

    const route = resolveRoute({
      text,
      activeRole: s?.activeRole ?? DEFAULT_ROLE_ALIAS,
      roleNames: roles().map((r) => r.roleName),
      isCustomRole: sessionIsCustomRole,
      defaultRoleAlias: DEFAULT_ROLE_ALIAS,
      defaultBackendRole: DEFAULT_BACKEND_ROLE,
    });

    if (route.error) { pushMessage("event", route.error); return; }
    if (originSessionId === activeSessionId()) applyRouteState(route);
    if (route.prefetchRole) prefetchRoleResources(route.prefetchRole);
    if (route.explicitRoleMention && !route.routedText) return;

    const { sendRoleLabel, isCommand, inRoleContext, isAppCommand, routedText } = route;

    if (!silent) {
      if (originSessionId) {
        appendMessageToSession(originSessionId, {
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: "user", text, at: now(),
        });
        maybeAutoTitleSession(originSessionId, text);
      } else {
        pushMessage("user", text);
      }
    }
    patchOriginSession({ submitting: true, status: "running", agentState: undefined, thoughtText: "" });

    if (await runAgentControlCommand(text, isCommand, inRoleContext, originSessionId, patchOriginSession)) return;

    const { appendOriginMessage, startOriginStream, completeOriginStream, dropOriginStream } =
      buildStreamOps(originSessionId, sendRoleLabel);

    let finalStatus: "done" | "error" = "done";
    if (!isAppCommand && originSessionId) startOriginStream();

    try {
      const res = await assistantApi.chat({
        input: routedText,
        runtimeKind: s?.runtimeKind ?? null,
        appSessionId: originSessionId ?? null,
      });
      if (runToken <= getCanceledRunToken()) { dropOriginStream(); return; }
      if (res.runtimeKind && originSessionId === activeSessionId()) setPreferredAssistant(res.runtimeKind);
      if (text.startsWith("/app_role")) void refreshRoles();

      if (!res.ok) {
        dropOriginStream();
        showToast(res.reply);
        appendOriginMessage({ id: `${now()}-err`, roleName: "event", text: res.reply, at: now() });
        finalStatus = "error";
        return;
      }
      if (!isAppCommand) {
        completeOriginStream(res.reply);
      } else {
        appendOriginMessage({
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: sendRoleLabel, text: res.reply, at: now(),
        });
      }
    } catch (e) {
      if (runToken <= getCanceledRunToken()) { dropOriginStream(); return; }
      dropOriginStream();
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) showToast(errMsg);
      appendOriginMessage({ id: `${now()}-err`, roleName: "event", text: errMsg, at: now() });
      finalStatus = "error";
      return;
    } finally {
      if (originSessionId) acceptingStreams.delete(originSessionId);
      if (runToken <= getCanceledRunToken()) {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued(originSessionId);
        return;
      }
      patchOriginSession({ submitting: false, status: finalStatus });
      runNextQueued(originSessionId);
    }
  };

  return { sendRaw, runNextQueued, cancelCurrentRun };
}

export type MessageSend = ReturnType<typeof useMessageSend>;
