import type { Accessor } from "solid-js";
import type { AppSession, AppSegment, AppToolCall, QueuedItem } from "../components/types";
import type { InputDeliveryCapabilities } from "../components/types";
import { now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS } from "../components/types";
import { appSessionApi, assistantApi } from "../lib/tauriApi";
import type { ImageAttachment } from "../lib/tauriApi";
import { parseAgentControlCommand, resolveRoute } from "../lib/chatPipeline";
import { shouldAutoTitleSession, computeAutoTitleForSession } from "../lib/sessionHelpers";
import { projectQueuedItemDequeue, queuedItemsFor } from "../lib/messageQueue";
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
  clearSessionStream?: (sid: string) => void;
  inputDelivery?: Accessor<InputDeliveryCapabilities | undefined>;
};

export function useMessageSend({
  sessionManager,
  streamEngine,
  agentContext,
  closeMentionMenu,
  closeSlashMenu,
  showToast,
  clearSessionStream,
  inputDelivery,
}: MessageSendDeps) {
  const {
    sessions, setSessions,
    activeSessionId, activeSession,
    updateSession, patchActiveSession,
    appendMessageToSession, pushMessage,
    scheduleScrollToBottom,
    getSessionIndex,
    ensureSessionPersisted,
  } = sessionManager;

  const {
    acceptingStreams,
    releaseStream,
    resetStreamState, finalizeSessionStream,
  } = streamEngine;

  const {
    roles,
    bumpRunToken, isRunCancelled,
    activeBackendRole,
    refreshRoles,
    fetchAndCacheAgentCommands,
    setPreferredAssistant,
    cancelCurrentRun: cancelCurrentRunBase,
  } = agentContext;

  // --- helpers ---

  const pushSessionEvent = (sessionId: string | null, text: string) => {
    if (sessionId) {
      appendMessageToSession(sessionId, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: "event",
        text,
        at: now(),
      });
    } else {
      pushMessage("event", text);
    }
  };

  const queuedItemKey = (item: QueuedItem): string => item.clientId ?? item.id;

  const patchSessionById = (sessionId: string | null, patch: Partial<AppSession>) => {
    if (!sessionId) return;
    updateSession(sessionId, patch);
  };

  const applyRouteState = (route: ReturnType<typeof resolveRoute>) => {
    if (!route.activateRole) return;
    const role = roles().find((r) => r.roleName === route.activateRole);
    // A persona owns its engine, same rule as the composer's persona switch: activating a
    // role by @mention has to move the runtime too, or the picker keeps showing the old
    // engine's name and model list under the new persona.
    patchActiveSession({
      activeRole: route.activateRole,
      ...(role?.runtimeKind
        ? { runtimeKind: role.runtimeKind, runtimeProfileId: role.runtimeProfileId ?? null }
        : {}),
      discoveredConfigOptions: [],
    });
  };

  // The option catalog is owned by App.tsx's (session, persona, engine) effect — activating
  // the role above moves that triple, which loads it. Only the command list, which is keyed
  // per (runtime, role) rather than per session, is prefetched here.
  const prefetchRoleResources = (roleName: string) => {
    const targetRole = roles().find((r) => r.roleName === roleName);
    if (!targetRole) return;
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
        pushSessionEvent(originSessionId, `cancelled ${role}`);
      } else {
        if (originSessionId) await assistantApi.setMode(role, cmd, originSessionId);
        pushSessionEvent(originSessionId, `${role} mode → ${cmd}`);
      }
    } catch (e) {
      pushSessionEvent(originSessionId, String(e));
    } finally {
      patchOriginSession({ submitting: false, status: "idle", turnPhase: "idle" });
      void runNextQueued(originSessionId);
    }
    return true;
  };

  // --- public API ---

  const queueRuns = new Set<string>();
  const steerRuns = new Set<string>();

  const runNextQueuedInternal = async (preferredSessionId?: string | null): Promise<void> => {
    const sid = preferredSessionId ?? activeSessionId();
    if (!sid) return;
    const idx = getSessionIndex(sid);
    const s = idx !== -1 ? sessions[idx] : null;
    if (s?.submitting) return;
    const itemQueue = queuedItemsFor(sessions, getSessionIndex, sid);
    const projectedItems = projectQueuedItemDequeue(itemQueue);
    const { merged, count } = {
      merged: projectedItems.merged,
      count: projectedItems.items.length,
    };
    if (count === 0 || !merged) {
      if (s?.turnPhase === "cancelling" || s?.turnPhase === "sending") {
        updateSession(sid, { submitting: false, status: "idle", turnPhase: "idle" });
      }
      return;
    }
    const claimedIds = new Set(projectedItems.items.map(queuedItemKey));
    const queuedAttachments = projectedItems.attachments;
    if (idx !== -1) {
      setSessions(idx, "queuedItems", (prev) => prev.map((item) => claimedIds.has(queuedItemKey(item)) ? { ...item, status: "claimed" } : item));
      updateSession(sid, {
        submitting: true,
        status: "running",
        turnPhase: "sending",
        agentState: "Sending queued message...",
      });
    }
    if (count > 1) {
      appendMessageToSession(sid, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: "event",
        text: `queued messages merged: ${count}`,
        at: now(),
      });
    }
    const queuedPrompt = projectedItems.roleName && projectedItems.roleName !== s?.activeRole && !merged.startsWith("@")
      ? `@${projectedItems.roleName} ${merged}`
      : merged;
    const backendIds = projectedItems.items
      .map((item) => item.id)
      .filter((id) => !id.startsWith("pending-"));
    let backendClaimed = false;
    try {
      if (backendIds.length > 0) {
        await appSessionApi.claimInbox(backendIds);
        backendClaimed = true;
      }
      const ok = await sendRaw(queuedPrompt, false, sid, queuedAttachments, queuedAttachments);
      if (!ok && backendClaimed) await appSessionApi.restoreInbox(backendIds);
      const current = getSessionIndex(sid);
      if (current === -1) return;
      if (ok) {
        for (const item of projectedItems.items) {
          if (!item.id.startsWith("pending-")) void appSessionApi.removeInbox(item.id).catch(() => {});
        }
        setSessions(current, "queuedItems", (prev) => prev.filter((item) => !claimedIds.has(queuedItemKey(item))));
      } else {
        setSessions(current, "queuedItems", (prev) => prev.map((item) => claimedIds.has(queuedItemKey(item)) ? { ...item, status: "queued" } : item));
        const currentSession = sessions[current];
        if (currentSession?.turnPhase === "sending" && currentSession.streamingRunToken === null) {
          updateSession(sid, { submitting: false, status: "error", turnPhase: "idle" });
        }
      }
    } catch (error) {
      if (backendClaimed) await appSessionApi.restoreInbox(backendIds).catch(() => {});
      const current = getSessionIndex(sid);
      if (current === -1) return;
      setSessions(current, "queuedItems", (prev) => prev.map((item) => claimedIds.has(queuedItemKey(item)) ? { ...item, status: "queued" } : item));
      const currentSession = sessions[current];
      if (currentSession?.turnPhase === "sending" && currentSession.streamingRunToken === null) {
        updateSession(sid, { submitting: false, status: "error", turnPhase: "idle" });
      }
      showToast(`Queued message was not sent: ${String(error)}`);
    }
  };

  const runNextQueued = async (preferredSessionId?: string | null): Promise<void> => {
    const sid = preferredSessionId ?? activeSessionId();
    if (!sid || queueRuns.has(sid)) return;
    queueRuns.add(sid);
    updateSession(sid, { queueRunActive: true });
    try {
      await runNextQueuedInternal(sid);
    } finally {
      queueRuns.delete(sid);
      updateSession(sid, { queueRunActive: false });
    }
  };

  const cancelCurrentRun = async () => {
    const sid = activeSessionId();
    if (!sid) return;
    await cancelCurrentRunBase(() => runNextQueued(sid), clearSessionStream);
  };

  const sendQueuedNow = async () => {
    const sid = activeSessionId();
    const idx = sid ? getSessionIndex(sid) : -1;
    const session = idx !== -1 ? sessions[idx] : null;
    if (!sid || !session) return;
    if (queueRuns.has(sid) || session.queueRunActive || session.turnPhase === "cancelling") return;
    if (!session.submitting) {
      await runNextQueued(sid);
      return;
    }
    const hasQueuedItems = queuedItemsFor(sessions, getSessionIndex, sid)
      .some((item) => item.status === "queued");
    if (!hasQueuedItems) return;
    if (inputDelivery?.()?.strategy === "steer") {
      const projected = projectQueuedItemDequeue(queuedItemsFor(sessions, getSessionIndex, sid));
      const merged = projected.merged;
      const attachments = projected.attachments;
      const queuedRole = projected.items[0]?.roleName ?? null;
      const sameRole = !queuedRole || queuedRole === session.activeRole;
      const ids = projected.items
        .map((item) => item.id)
        .filter((id) => !id.startsWith("pending-"));
      if (merged && sameRole && attachments.length === 0 && !steerRuns.has(sid)) {
        steerRuns.add(sid);
        updateSession(sid, { queueRunActive: true, agentState: "Steering current turn…" });
        let claimed = false;
        try {
          if (ids.length > 0) {
            await appSessionApi.claimInbox(ids);
            claimed = true;
          }
          await assistantApi.steerSession(session.activeRole || activeBackendRole(), sid, merged);
          appendMessageToSession(sid, {
            id: `${now()}-${Math.random().toString(36).slice(2)}`,
            roleName: "user",
            text: merged,
            at: now(),
          });
          for (const id of ids) void appSessionApi.removeInbox(id).catch(() => {});
          const current = getSessionIndex(sid);
          if (current !== -1) {
            if (projected.items.length > 0) {
              const claimedKeys = new Set(projected.items.map(queuedItemKey));
              setSessions(current, "queuedItems", (prev) => prev.filter((item) => !claimedKeys.has(queuedItemKey(item))));
            }
            updateSession(sid, { submitting: true, status: "running", turnPhase: "running" });
          }
          return;
        } catch (error) {
          if (claimed) await appSessionApi.restoreInbox(ids).catch(() => {});
          // A provider can finish in the small window between the UI check and the steer
          // write. In that case the regular cancel-and-drain path is the safe retry.
          void error;
        } finally {
          steerRuns.delete(sid);
          updateSession(sid, { queueRunActive: false });
        }
      }
    }
    await cancelCurrentRunBase(() => runNextQueued(sid), clearSessionStream, "sendQueued");
  };

  const sendRaw = async (text: string, silent = false, targetSessionId?: string | null, attachments?: ImageAttachment[], sourceImages?: ImageAttachment[]): Promise<boolean> => {
    const runToken = bumpRunToken();
    const originSessionId = targetSessionId ?? activeSessionId();
    closeMentionMenu();
    closeSlashMenu();

    // A draft session (opened project / new session / persona switch with nothing sent yet)
    // has no `app_sessions` row. This is the one place every real send funnels through, so
    // it's the only place that needs to create it — never on switch, only on an actual send.
    if (originSessionId) {
      try {
        await ensureSessionPersisted(originSessionId);
      } catch (e) {
        showToast(`Could not start session: ${String(e)}`);
        return false;
      }
    }

    const patchOriginSession = (patch: Partial<AppSession>) => patchSessionById(originSessionId, patch);

    const _oidx = originSessionId ? getSessionIndex(originSessionId) : -1;
    const s = (_oidx !== -1 ? sessions[_oidx] : null) ?? activeSession();
    const sessionIsCustomRole = true;

    const route = resolveRoute({
      text,
      activeRole: s?.activeRole ?? DEFAULT_ROLE_ALIAS,
      roleNames: roles().map((r) => r.roleName),
      isCustomRole: sessionIsCustomRole,
      defaultRoleAlias: DEFAULT_ROLE_ALIAS,
      defaultBackendRole: DEFAULT_BACKEND_ROLE,
    });

    if (route.error) { pushMessage("event", route.error); return false; }
    if (originSessionId === activeSessionId()) applyRouteState(route);
    if (route.prefetchRole) prefetchRoleResources(route.prefetchRole);
    if (route.explicitRoleMention && !route.routedText) return false;

    const { sendRoleLabel, isCommand, inRoleContext, isAppCommand, routedText } = route;

    if (!silent) {
      if (originSessionId) {
        appendMessageToSession(originSessionId, {
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: "user", text, at: now(),
          images: (sourceImages ?? attachments)?.length ? (sourceImages ?? attachments)!.map((a) => ({ data: a.data, mimeType: a.mimeType })) : undefined,
        });
        maybeAutoTitleSession(originSessionId, text);
      } else {
        pushMessage("user", text);
      }
    }
    patchOriginSession({ submitting: true, status: "running", turnPhase: "running", agentState: undefined, thoughtText: "" });

    if (await runAgentControlCommand(text, isCommand, inRoleContext, originSessionId, patchOriginSession)) return true;

    const stream = openStreamSession(originSessionId, sendRoleLabel, runToken);

    let finalStatus: "done" | "error" = "done";
    if (!isAppCommand && originSessionId) stream.start();

    try {
      const res = await assistantApi.chat({
        input: routedText,
        runtimeKind: s?.runtimeKind ?? null,
        appSessionId: originSessionId ?? null,
        attachments: attachments && attachments.length > 0 ? attachments : undefined,
      });
      if (isRunCancelled(runToken)) { return false; }
      if (res.runtimeKind && originSessionId === activeSessionId()) setPreferredAssistant(res.runtimeKind);
      if (text.startsWith("/app_role")) void refreshRoles(s?.projectId ?? undefined);

      const hasPerRoleReplies = (res.roleReplies?.length ?? 0) > 1;
      if (!res.ok && !hasPerRoleReplies) {
        const liveSession = originSessionId ? sessions[getSessionIndex(originSessionId)] : null;
        const hasStreamContent = Boolean(
          liveSession?.streamingMessage?.text?.trim() ||
          (liveSession?.toolCalls && Object.keys(liveSession.toolCalls).length > 0) ||
          (liveSession?.streamSegments && liveSession.streamSegments.length > 0)
        );
        if (hasStreamContent) {
          stream.complete();
        } else {
          stream.drop();
        }
        const isCancel = res.reply.toLowerCase().includes("cancel") || isRunCancelled(runToken);
        if (!isCancel) {
          showToast(res.reply);
          stream.appendMessage({ id: `${now()}-err`, roleName: "event", text: res.reply, at: now() });
        }
        finalStatus = "error";
        return false;
      }
      if (!isAppCommand) {
        if (res.roleReplies && res.roleReplies.length > 1 && originSessionId) {
          const liveSession = sessions[getSessionIndex(originSessionId)];
          const segmentsByRole = new Map<string, AppSegment[]>();
          const toolsByRole = new Map<string, AppToolCall[]>();
          for (const segment of liveSession?.streamSegments ?? []) {
            const roleName = segment.roleName ?? (segment.kind === "tool" ? segment.tc.roleName : undefined);
            if (!roleName) continue;
            const segments = segmentsByRole.get(roleName) ?? [];
            segments.push(segment);
            segmentsByRole.set(roleName, segments);
          }
          for (const tool of Object.values(liveSession?.toolCalls ?? {})) {
            if (!tool.roleName) continue;
            const tools = toolsByRole.get(tool.roleName) ?? [];
            tools.push(tool);
            toolsByRole.set(tool.roleName, tools);
          }
          stream.drop();
          for (const r of res.roleReplies) {
            const roleSegments = segmentsByRole.get(r.roleName);
            appendMessageToSession(originSessionId, {
              id: `${now()}-${Math.random().toString(36).slice(2)}`,
              roleName: r.roleName,
              text: r.reply,
              at: now(),
              segments: roleSegments && roleSegments.length > 0 ? roleSegments : undefined,
              toolCalls: toolsByRole.get(r.roleName),
            });
          }
          if (!res.ok) showToast("One or more agents failed; successful agent replies were preserved.", "error");
        } else {
          stream.complete(res.reply);
        }
      } else {
        stream.appendMessage({
          id: `${now()}-${Math.random().toString(36).slice(2)}`,
          roleName: sendRoleLabel, text: res.reply, at: now(),
        });
      }
    } catch (e) {
      if (isRunCancelled(runToken)) { return false; }
      const liveSession = originSessionId ? sessions[getSessionIndex(originSessionId)] : null;
      const hasStreamContent = Boolean(
        liveSession?.streamingMessage?.text?.trim() ||
        (liveSession?.toolCalls && Object.keys(liveSession.toolCalls).length > 0) ||
        (liveSession?.streamSegments && liveSession.streamSegments.length > 0)
      );
      if (hasStreamContent) {
        stream.complete();
      } else {
        stream.drop();
      }
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) {
        showToast(errMsg);
        stream.appendMessage({ id: `${now()}-err`, roleName: "event", text: errMsg, at: now() });
      }
      finalStatus = "error";
      return false;
    } finally {
      // Token-guarded release: only clears the acceptingStreams slot if this run
      // still owns it. A newer run that already called stream.start() will have
      // overwritten the token, so releaseStream() becomes a no-op.
      if (originSessionId) releaseStream(originSessionId, runToken);
      if (isRunCancelled(runToken)) {
        // Cancelled run: runNextQueued() was already called by cancelCurrentRun().
        return false;
      }
      patchOriginSession({ submitting: false, status: finalStatus, turnPhase: "idle" });
      void runNextQueued(originSessionId);
    }
    return finalStatus === "done";
  };

  return { sendRaw, runNextQueued, cancelCurrentRun, sendQueuedNow };
}

export type MessageSend = ReturnType<typeof useMessageSend>;
