import { produce } from "solid-js/store";
import { now } from "../components/types";
import { MAX_THOUGHT_CHARS } from "../lib/sessionHelpers";
import type { SessionManager } from "./useSessionManager";

export function useStreamEngine(sessionManager: SessionManager) {
  const {
    sessions,
    setSessions,
    activeSessionId,
    patchActiveSession,
    appendMessageToSession,
    updateSession,
    scheduleScrollToBottom,
    getSessionIndex,
  } = sessionManager;

  // Map<sessionId, runToken> — tracks which run currently owns a session's stream.
  // Use acceptingStreams.set(sid, runToken) to open, and releaseStream(sid, runToken)
  // to close only if the token still matches (prevents a finishing run from evicting
  // a newer run that already re-opened the same session's stream).
  const acceptingStreams = new Map<string, number>();

  const releaseStream = (sid: string, runToken: number) => {
    if (acceptingStreams.get(sid) === runToken) {
      acceptingStreams.delete(sid);
    }
  };
  const normalizeNewlines = (input: string): string => input.replace(/\r\n?/g, "\n");

  const normalizeToolLocations = (
    raw: unknown[] | undefined,
  ): Array<{ path: string; line?: number }> | undefined => {
    if (!Array.isArray(raw) || raw.length === 0) return undefined;
    const out = raw
      .map((item) => {
        if (!item || typeof item !== "object") return null;
        const obj = item as Record<string, unknown>;
        const path = typeof obj.path === "string" ? obj.path : "";
        if (!path) return null;
        const line = typeof obj.line === "number" ? obj.line : undefined;
        return { path, line };
      })
      .filter((item): item is NonNullable<typeof item> => !!item);
    return out.length > 0 ? out : undefined;
  };

  const checkpointTimers = new Map<string, number>();

  const scheduleCheckpoint = (sessionId: string, delayMs = 1500) => {
    if (checkpointTimers.has(sessionId)) return;
    const timer = window.setTimeout(() => {
      checkpointTimers.delete(sessionId);
      sessionManager.checkpointStreamingMessage(sessionId);
    }, delayMs);
    checkpointTimers.set(sessionId, timer);
  };

  const cancelCheckpoint = (sessionId: string) => {
    const t = checkpointTimers.get(sessionId);
    if (t !== undefined) {
      window.clearTimeout(t);
      checkpointTimers.delete(sessionId);
    }
  };

  const appendStream = (sessionId: string, chunk: string, roleName?: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized) return;
    const idx = getSessionIndex(sessionId);
    if (idx === -1) return;
    setSessions(idx, produce((s) => {
      if (!s.streamingMessage) return;
      s.streamingMessage.text = (s.streamingMessage.text ?? "") + normalized;
      const last = s.streamSegments[s.streamSegments.length - 1];
      if (last?.kind === "text" && last.roleName === roleName) {
        last.text += normalized;
      } else {
        s.streamSegments.push({ kind: "text", text: normalized, roleName });
      }
    }));
    scheduleScrollToBottom();
    scheduleCheckpoint(sessionId);
  };

  const appendThought = (sessionId: string, chunk: string, roleName?: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized.trim()) return;
    const idx = getSessionIndex(sessionId);
    if (idx === -1) return;
    setSessions(idx, produce((s) => {
      const next = `${s.thoughtText ?? ""}${normalized}`;
      s.thoughtText = next.length <= MAX_THOUGHT_CHARS ? next : next.slice(next.length - MAX_THOUGHT_CHARS);
      const last = s.streamSegments[s.streamSegments.length - 1];
      if (last?.kind === "thought" && last.roleName === roleName) {
        last.text += normalized;
      } else {
        s.streamSegments.push({ kind: "thought", text: normalized, roleName });
      }
    }));
    scheduleScrollToBottom();
    scheduleCheckpoint(sessionId);
  };

  const resetStreamState = (sessionId?: string) => {
    if (sessionId) {
      cancelCheckpoint(sessionId);
      acceptingStreams.delete(sessionId); // unconditional reset (e.g. session close)
    } else {
      for (const sid of checkpointTimers.keys()) {
        cancelCheckpoint(sid);
      }
      acceptingStreams.clear();
    }
  };

  const dropStream = () => {
    const sid = activeSessionId();
    if (sid) cancelCheckpoint(sid);
    patchActiveSession({ streamingMessage: null });
    resetStreamState(sid ?? undefined);
  };

  /**
   * Finalize the live stream into a persistent message.
   *
   * @param expectedRunToken — if provided, the call is a no-op when the session's
   *   streamingRunToken no longer matches. Guards against a late-arriving response
   *   from a cancelled run overwriting a newer run's live stream (the exact bug
   *   that caused "UI doesn't update after cancel + queued send").
   */
  const finalizeSessionStream = (sessionId: string, fallbackRoleName: string, finalReply?: string, expectedRunToken?: number) => {
    cancelCheckpoint(sessionId);
    const idx = getSessionIndex(sessionId);
    const sess = idx !== -1 ? sessions[idx] : undefined;
    if (
      expectedRunToken !== undefined &&
      sess?.streamingRunToken !== null &&
      sess?.streamingRunToken !== undefined &&
      sess.streamingRunToken !== expectedRunToken
    ) {
      return; // a newer run owns the stream — don't clobber it
    }
    const row = sess?.streamingMessage ?? null;
    const snapshotToolCalls = sess && Object.keys(sess.toolCalls).length > 0 ? Object.values(sess.toolCalls) : undefined;
    let snapshotSegments = sess && sess.streamSegments.length > 0 ? [...sess.streamSegments] : undefined;
    if (!snapshotSegments && snapshotToolCalls && snapshotToolCalls.length > 0) {
      snapshotSegments = snapshotToolCalls.map((tc) => ({ kind: "tool" as const, tc, roleName: tc.roleName || fallbackRoleName }));
    }
    if (snapshotSegments && finalReply) {
      const last = snapshotSegments[snapshotSegments.length - 1];
      if (last && last.kind === "text") {
        snapshotSegments[snapshotSegments.length - 1] = { kind: "text", text: normalizeNewlines(finalReply), roleName: last.roleName };
      } else {
        snapshotSegments.push({ kind: "text", text: normalizeNewlines(finalReply) });
      }
    }
    if (row) {
      const text = normalizeNewlines(finalReply ?? row.text);
      const snapshotThought = sess?.thoughtText || undefined;
      // Always append if there was an active streamingMessage — dropping it
      // silently causes the response to disappear (e.g. after cancel + queued
      // send when the agent returns empty text for the interrupted turn).
      appendMessageToSession(sessionId, {
        ...row,
        text,
        at: now(),
        toolCalls: snapshotToolCalls,
        segments: snapshotSegments,
        thoughtText: snapshotThought,
      });
      updateSession(sessionId, { streamingMessage: null, streamingRunToken: null, thoughtText: "" });
    } else if ((finalReply && finalReply.trim()) || (snapshotToolCalls && snapshotToolCalls.length > 0)) {
      appendMessageToSession(sessionId, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: fallbackRoleName,
        text: normalizeNewlines(finalReply ?? ""),
        at: now(),
        toolCalls: snapshotToolCalls,
        segments: snapshotSegments,
      });
    }
    resetStreamState(sessionId);
    updateSession(sessionId, {
      toolCalls: {},
      streamSegments: [],
      currentPlan: null,
      pendingPermissions: [],
      agentState: undefined,
      thoughtText: "",
    });
  };

  return {
    acceptingStreams,
    releaseStream,
    appendStream,
    appendThought,
    resetStreamState,
    dropStream,
    finalizeSessionStream,
    normalizeToolLocations,
    scheduleCheckpoint,
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
