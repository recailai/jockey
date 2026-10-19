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

  // Deltas arrive far faster than frames. Writing each one into the store re-rendered (and
  // re-parsed the markdown of) the whole growing message per token, so they are buffered per
  // session and applied in one store write per flush. A timer, not rAF: rAF stops while the
  // window is hidden, and the buffer would then grow until it was shown again.
  type PendingChunk = { kind: "text" | "thought"; text: string; roleName?: string };
  const FLUSH_MS = 33;
  const pending = new Map<string, PendingChunk[]>();
  let flushTimer: number | null = null;

  const applyChunks = (sessionId: string, chunks: PendingChunk[]) => {
    const idx = getSessionIndex(sessionId);
    if (idx === -1) return;
    setSessions(idx, produce((s) => {
      for (const chunk of chunks) {
        if (chunk.kind === "text") {
          if (!s.streamingMessage) continue;
          s.streamingMessage.text = (s.streamingMessage.text ?? "") + chunk.text;
        } else {
          const next = `${s.thoughtText ?? ""}${chunk.text}`;
          s.thoughtText = next.length <= MAX_THOUGHT_CHARS ? next : next.slice(next.length - MAX_THOUGHT_CHARS);
        }
        const last = s.streamSegments[s.streamSegments.length - 1];
        if (last?.kind === chunk.kind && last.roleName === chunk.roleName) {
          last.text += chunk.text;
        } else {
          s.streamSegments.push({ kind: chunk.kind, text: chunk.text, roleName: chunk.roleName });
        }
      }
    }));
    scheduleScrollToBottom();
  };

  /** Apply buffered text/thought now. Called before any other stream event so a tool call
   *  never lands ahead of text that streamed before it. */
  const flushStream = (sessionId?: string) => {
    const ids = sessionId ? [sessionId] : [...pending.keys()];
    for (const sid of ids) {
      const chunks = pending.get(sid);
      if (!chunks) continue;
      pending.delete(sid);
      applyChunks(sid, chunks);
    }
  };

  const enqueueChunk = (sessionId: string, chunk: PendingChunk) => {
    const chunks = pending.get(sessionId) ?? [];
    const last = chunks[chunks.length - 1];
    if (last && last.kind === chunk.kind && last.roleName === chunk.roleName) last.text += chunk.text;
    else chunks.push(chunk);
    pending.set(sessionId, chunks);
    if (flushTimer === null) {
      flushTimer = window.setTimeout(() => {
        flushTimer = null;
        flushStream();
      }, FLUSH_MS);
    }
  };

  const appendStream = (sessionId: string, chunk: string, roleName?: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized) return;
    enqueueChunk(sessionId, { kind: "text", text: normalized, roleName });
  };

  const appendThought = (sessionId: string, chunk: string, roleName?: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized.trim()) return;
    enqueueChunk(sessionId, { kind: "thought", text: normalized, roleName });
  };

  const resetStreamState = (sessionId?: string) => {
    if (sessionId) {
      pending.delete(sessionId);
      acceptingStreams.delete(sessionId); // unconditional reset (e.g. session close)
    } else {
      pending.clear();
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
    flushStream,
    normalizeToolLocations,
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
