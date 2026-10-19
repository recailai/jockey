import { produce } from "solid-js/store";
import type { AppMessage } from "../components/types";
import { MAX_THOUGHT_CHARS } from "../lib/sessionHelpers";
import type { SessionManager } from "./useSessionManager";

export function useStreamEngine(sessionManager: SessionManager) {
  const {
    sessions,
    setSessions,
    activeSessionId,
    patchActiveSession,
    adoptMessages,
    updateSession,
    scheduleScrollToBottom,
    getSessionIndex,
  } = sessionManager;

  // Map<sessionId, runToken> — which turn currently owns a session's live stream. Opened by
  // `turn/started` (engineEvents) and cleared by `settleSessionStream`.
  const acceptingStreams = new Map<string, number>();

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

  return {
    acceptingStreams,
    appendStream,
    appendThought,
    resetStreamState,
    dropStream,
    settleSessionStream,
    normalizeToolLocations,
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
