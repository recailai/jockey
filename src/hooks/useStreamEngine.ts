import { produce } from "solid-js/store";
import { now } from "../components/types";
import type { AppMessage } from "../components/types";
import { MAX_THOUGHT_CHARS } from "../lib/sessionHelpers";
import type { SessionManager } from "./useSessionManager";

export function useStreamEngine(sessionManager: SessionManager) {
  const {
    sessions,
    setSessions,
    activeSessionId,
    patchActiveSession,
    appendMessageLocal,
    adoptMessages,
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
  };

  const resetStreamState = (sessionId?: string) => {
    if (sessionId) {
      acceptingStreams.delete(sessionId); // unconditional reset (e.g. session close)
    } else {
      acceptingStreams.clear();
    }
  };

  const dropStream = () => {
    const sid = activeSessionId();
    patchActiveSession({ streamingMessage: null });
    resetStreamState(sid ?? undefined);
  };

  const clearLiveTurn = (sessionId: string) => {
    resetStreamState(sessionId);
    updateSession(sessionId, {
      streamingMessage: null,
      streamingRunToken: null,
      toolCalls: {},
      streamSegments: [],
      currentPlan: null,
      pendingPermissions: [],
      agentState: undefined,
      thoughtText: "",
    });
  };

  /**
   * Replace the live projection with the turn's persisted messages from the engine.
   * Messages are adopted regardless; the live state is only cleared if this run still owns
   * it, so a late response from a cancelled run never wipes a newer run's stream.
   */
  const settleSessionStream = (sessionId: string, messages: readonly AppMessage[], expectedRunToken?: number) => {
    adoptMessages(sessionId, messages);
    const idx = getSessionIndex(sessionId);
    const sess = idx !== -1 ? sessions[idx] : undefined;
    if (
      expectedRunToken !== undefined &&
      sess?.streamingRunToken !== null &&
      sess?.streamingRunToken !== undefined &&
      sess.streamingRunToken !== expectedRunToken
    ) {
      return;
    }
    clearLiveTurn(sessionId);
  };

  /**
   * Freeze the live stream into a displayed message (cancel, or a failed IPC call). Display
   * only: the engine records the same turn under the same message id.
   *
   * @param expectedRunToken — if provided, the call is a no-op when the session's
   *   streamingRunToken no longer matches. Guards against a late-arriving response
   *   from a cancelled run overwriting a newer run's live stream (the exact bug
   *   that caused "UI doesn't update after cancel + queued send").
   */
  const finalizeSessionStream = (sessionId: string, fallbackRoleName: string, finalReply?: string, expectedRunToken?: number) => {
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
      appendMessageLocal(sessionId, {
        ...row,
        text,
        at: now(),
        toolCalls: snapshotToolCalls,
        segments: snapshotSegments,
        thoughtText: snapshotThought,
      });
      updateSession(sessionId, { streamingMessage: null, streamingRunToken: null, thoughtText: "" });
    } else if ((finalReply && finalReply.trim()) || (snapshotToolCalls && snapshotToolCalls.length > 0)) {
      appendMessageLocal(sessionId, {
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
    settleSessionStream,
    normalizeToolLocations,
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
