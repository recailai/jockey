import type { AppMessage } from "../components/types";
import { now } from "../components/types";
import { engineApi } from "../lib/tauriApi";
import { createClientId } from "../lib/clientId";
import type { ImageAttachment } from "../lib/tauriApi";
import type { SessionManager } from "./useSessionManager";

type MessageSendDeps = {
  sessionManager: SessionManager;
  closeMentionMenu: () => void;
  closeSlashMenu: () => void;
  showToast: (msg: string, severity?: "error" | "info") => void;
};

/**
 * Input from this window, handed to the engine. The engine decides whether it starts a turn,
 * joins the queue, or runs as a control command, and routes `@mentions`; this hook only shows
 * an optimistic bubble and reports problems. Turn progress arrives through `engineEvents`.
 */
export function useMessageSend({
  sessionManager,
  closeMentionMenu,
  closeSlashMenu,
  showToast,
}: MessageSendDeps) {
  const {
    sessions, setSessions,
    activeSessionId,
    updateSession,
    appendMessageLocal, pushMessage,
    getSessionIndex,
    ensureSessionPersisted,
  } = sessionManager;

  const pushSessionEvent = (sessionId: string, text: string) => {
    appendMessageLocal(sessionId, {
      id: `${now()}-${Math.random().toString(36).slice(2)}`,
      roleName: "event",
      text,
      at: now(),
    });
  };

  const removeMessage = (sessionId: string, id: string) => {
    const idx = getSessionIndex(sessionId);
    if (idx !== -1) setSessions(idx, "messages", (msgs) => msgs.filter((m) => m.id !== id));
  };

  const sendRaw = async (
    text: string,
    _silent = false,
    targetSessionId?: string | null,
    attachments?: ImageAttachment[],
  ): Promise<boolean> => {
    closeMentionMenu();
    closeSlashMenu();
    const sid = targetSessionId ?? activeSessionId();
    if (!sid) {
      pushMessage("event", "Open a session first.");
      return false;
    }
    // A draft session (opened project / new session with nothing sent yet) has no row; this is
    // the one place every real send funnels through, so it is the only place that creates it.
    try {
      await ensureSessionPersisted(sid);
    } catch (e) {
      showToast(`Could not start session: ${String(e)}`);
      return false;
    }
    const idx = getSessionIndex(sid);
    const session = idx !== -1 ? sessions[idx] : null;

    // Busy sessions queue the input instead; its chip appears via `queue/changed`.
    const userMessageId = createClientId();
    const optimistic = !session?.submitting;
    if (optimistic) {
      const userMessage: AppMessage = {
        id: userMessageId,
        roleName: "user",
        text,
        at: now(),
        images: attachments?.length ? attachments.map((a) => ({ data: a.data, mimeType: a.mimeType })) : undefined,
      };
      appendMessageLocal(sid, userMessage);
      updateSession(sid, { submitting: true, status: "running", turnPhase: "sending" });
    }

    try {
      const res = await engineApi.submit({
        sessionId: sid,
        text,
        attachments: attachments && attachments.length > 0 ? attachments : undefined,
        userMessageId,
        assistantMessageId: createClientId(),
        runtimeKind: session?.runtimeKind ?? null,
      });
      if (res.status === "queued" && optimistic) {
        // The engine had a turn running that this window had not heard about yet.
        removeMessage(sid, userMessageId);
      }
      if (res.status === "control") {
        pushSessionEvent(sid, res.message);
        updateSession(sid, { submitting: false, status: "idle", turnPhase: "idle" });
      }
      return true;
    } catch (e) {
      if (optimistic) {
        removeMessage(sid, userMessageId);
        updateSession(sid, { submitting: false, status: "error", turnPhase: "idle" });
      }
      const message = String(e);
      showToast(message);
      pushSessionEvent(sid, message);
      return false;
    }
  };

  const cancelCurrentRun = async () => {
    const sid = activeSessionId();
    if (!sid) return;
    updateSession(sid, { turnPhase: "cancelling", agentState: "Cancelling…" });
    pushSessionEvent(sid, "Cancellation requested.");
    try {
      await engineApi.cancel(sid);
    } catch (error) {
      pushSessionEvent(sid, `Cancellation did not drain cleanly: ${String(error)}`);
      updateSession(sid, { agentState: "Cancellation did not complete; reconnect before sending." });
    }
  };

  const sendQueuedNow = async () => {
    const sid = activeSessionId();
    if (!sid) return;
    try {
      await engineApi.sendQueuedNow(sid);
    } catch (error) {
      showToast(`Queued message was not sent: ${String(error)}`);
    }
  };

  return { sendRaw, cancelCurrentRun, sendQueuedNow };
}

export type MessageSend = ReturnType<typeof useMessageSend>;
