import type { AppSession } from "../components/types";
import { now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS } from "../components/types";
import { appSessionApi, assistantApi } from "../lib/tauriApi";
import { parseAgentControlCommand, resolveRoute } from "../lib/chatPipeline";
import { shouldAutoTitleSession, computeAutoTitleForSession } from "../lib/sessionHelpers";
import { queuedInputsFor as queuedInputsFromStore, projectNextDequeue } from "../lib/messageQueue";
import { createStreamSession } from "../lib/streamSession";
import type { RunToken } from "../lib/runToken";
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
    releaseStream,
    appendThought: _appendThought,
    resetStreamState, finalizeSessionStream,
  } = streamEngine;

  const {
    roles,
    bumpRunToken, isRunCancelled,
    isCustomRole, activeBackendRole,
    refreshRoles,
    fetchConfigOptions, fetchAndCacheAgentCommands,
    setPreferredAssistant,
    cancelCurrentRun: cancelCurrentRunBase,
  } = agentContext;

  // --- helpers ---

  const queuedInputsFor = (sid: string | null): readonly string[] =>
    queuedInputsFromStore(sessions, getSessionIndex, sid);

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
    if (!sess || !shouldAutoTitleSession(sess)) return;
    const autoTitle = computeAutoTitleForSession(sess, sessions, text);
    updateSession(sid, { title: autoTitle });
    void appSessionApi.update(sid, { title: autoTitle }).catch(() => {});
  };

  const openStreamSession = (originSessionId: string | null, roleLabel: string, runToken: RunToken) =>
    createStreamSession(originSessionId, roleLabel, runToken, {
      appendMessageToSession,
      patchSession: patchSessionById as (sid: string, patch: Partial<AppSession>) => void,
      finalizeSessionStream,
      resetStreamState,
      scheduleScrollToBottom,
      getSession: (sid) => {
        const idx = getSessionIndex(sid);
        return idx !== -1 ? sessions[idx] : null;
      },
      acceptingStreams,
    });

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
    const { merged, count } = projectNextDequeue(queue);
    if (count === 0 || !merged) return;
    if (idx !== -1) setSessions(idx, "queuedMessages", []);
    if (count > 1) {
      appendMessageToSession(sid, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: "event",
        text: `queued messages merged: ${count}`,
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

    const stream = openStreamSession(originSessionId, sendRoleLabel, runToken);

    let finalStatus: "done" | "error" = "done";
    if (!isAppCommand && originSessionId) stream.start();

    try {
      const res = await assistantApi.chat({
        input: routedText,
        runtimeKind: s?.runtimeKind ?? null,
        appSessionId: originSessionId ?? null,
      });
      if (isRunCancelled(runToken)) { return; }
      if (res.runtimeKind && originSessionId === activeSessionId()) setPreferredAssistant(res.runtimeKind);
      if (text.startsWith("/app_role")) void refreshRoles();

      if (!res.ok) {
        stream.drop();
        showToast(res.reply);
        stream.appendMessage({ id: `${now()}-err`, roleName: "event", text: res.reply, at: now() });
        finalStatus = "error";
        return;
      }
      if (!isAppCommand) {
        stream.complete(res.reply);
      } else {
        stream.appendMessage({
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: sendRoleLabel, text: res.reply, at: now(),
        });
      }
    } catch (e) {
      if (isRunCancelled(runToken)) { return; }
      stream.drop();
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) showToast(errMsg);
      stream.appendMessage({ id: `${now()}-err`, roleName: "event", text: errMsg, at: now() });
      finalStatus = "error";
      return;
    } finally {
      // Token-guarded release: only clears the acceptingStreams slot if this run
      // still owns it. A newer run that already called stream.start() will have
      // overwritten the token, so releaseStream() becomes a no-op.
      if (originSessionId) releaseStream(originSessionId, runToken);
      if (isRunCancelled(runToken)) {
        // Cancelled run: runNextQueued() was already called by cancelCurrentRun().
        return;
      }
      patchOriginSession({ submitting: false, status: finalStatus });
      runNextQueued(originSessionId);
    }
  };

  return { sendRaw, runNextQueued, cancelCurrentRun };
}

export type MessageSend = ReturnType<typeof useMessageSend>;
