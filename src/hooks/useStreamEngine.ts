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
    pushMessage,
    scheduleScrollToBottom,
  } = sessionManager;

  const acceptingStreams = new Set<string>();
  const streamBatchBuffers = new Map<string, string>();
  const thoughtBatchBuffers = new Map<string, string>();
  let streamBatchRaf: number | null = null;

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

  const flushStreamBatch = () => {
    for (const [sid, buf] of streamBatchBuffers) {
      if (!buf) continue;
      const chunk = buf;
      streamBatchBuffers.set(sid, "");
      setSessions(
        (s) => s.id === sid && !!s.streamingMessage,
        produce((s) => {
          s.streamingMessage!.text = (s.streamingMessage!.text ?? "") + chunk;
          const last = s.streamSegments[s.streamSegments.length - 1];
          if (last && last.kind === "text") {
            s.streamSegments[s.streamSegments.length - 1] = { kind: "text" as const, text: last.text + chunk };
          } else {
            s.streamSegments.push({ kind: "text" as const, text: chunk });
          }
        }),
      );
    }
    for (const [sid, buf] of thoughtBatchBuffers) {
      if (!buf) continue;
      const chunk = buf;
      thoughtBatchBuffers.set(sid, "");
      setSessions(
        (s) => s.id === sid,
        "thoughtText",
        (prev) => {
          const next = `${prev ?? ""}${chunk}`;
          return next.length <= MAX_THOUGHT_CHARS ? next : next.slice(next.length - MAX_THOUGHT_CHARS);
        },
      );
    }
    scheduleScrollToBottom();
    streamBatchRaf = null;
  };

  const appendStream = (sessionId: string, chunk: string) => {
    if (!chunk) return;
    const existing = streamBatchBuffers.get(sessionId) ?? "";
    streamBatchBuffers.set(sessionId, existing + normalizeNewlines(chunk));
    if (streamBatchRaf === null) {
      streamBatchRaf = window.requestAnimationFrame(flushStreamBatch);
    }
  };

  const appendThought = (sessionId: string, chunk: string) => {
    const normalized = normalizeNewlines(chunk);
    if (!normalized.trim()) return;
    const existing = thoughtBatchBuffers.get(sessionId) ?? "";
    thoughtBatchBuffers.set(sessionId, existing + normalized);
    if (streamBatchRaf === null) {
      streamBatchRaf = window.requestAnimationFrame(flushStreamBatch);
    }
  };

  const resetStreamState = (sessionId?: string) => {
    if (streamBatchRaf !== null) {
      window.cancelAnimationFrame(streamBatchRaf);
      streamBatchRaf = null;
    }
    if (sessionId) {
      streamBatchBuffers.delete(sessionId);
      thoughtBatchBuffers.delete(sessionId);
      acceptingStreams.delete(sessionId);
    } else {
      streamBatchBuffers.clear();
      thoughtBatchBuffers.clear();
      acceptingStreams.clear();
    }
  };

  const dropStream = () => {
    patchActiveSession({ streamingMessage: null });
    resetStreamState(activeSessionId() ?? undefined);
  };

  const finalizeSessionStream = (sessionId: string, fallbackRoleName: string, finalReply?: string) => {
    flushStreamBatch();
    const sess = sessions.find((x) => x.id === sessionId);
    const row = sess?.streamingMessage ?? null;
    const snapshotToolCalls = sess && Object.keys(sess.toolCalls).length > 0 ? Object.values(sess.toolCalls) : undefined;
    const snapshotSegments = sess && sess.streamSegments.length > 0 ? [...sess.streamSegments] : undefined;
    if (snapshotSegments && finalReply) {
      const last = snapshotSegments[snapshotSegments.length - 1];
      if (last && last.kind === "text") {
        snapshotSegments[snapshotSegments.length - 1] = { kind: "text", text: normalizeNewlines(finalReply) };
      } else {
        snapshotSegments.push({ kind: "text", text: normalizeNewlines(finalReply) });
      }
    }
    if (row) {
      const text = normalizeNewlines(finalReply ?? row.text);
      const shouldAppend = !!text.trim() || !!snapshotToolCalls?.length || !!snapshotSegments?.length;
      if (shouldAppend) {
        appendMessageToSession(sessionId, {
          ...row,
          text,
          at: now(),
          toolCalls: snapshotToolCalls,
          segments: snapshotSegments,
        });
      }
      updateSession(sessionId, { streamingMessage: null, thoughtText: "" });
    } else if (finalReply && finalReply.trim()) {
      appendMessageToSession(sessionId, {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: fallbackRoleName,
        text: normalizeNewlines(finalReply),
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
      pendingPermission: null,
      agentState: undefined,
      thoughtText: "",
    });
  };

  let pendingSessionEvents: string[] = [];
  let sessionEventFlushTimer: number | null = null;

  const scheduleSessionEventFlush = () => {
    if (sessionEventFlushTimer !== null) return;
    sessionEventFlushTimer = window.setTimeout(() => {
      sessionEventFlushTimer = null;
      if (pendingSessionEvents.length === 0) return;
      pushMessage("event", pendingSessionEvents.join("\n"));
      pendingSessionEvents = [];
    }, 120);
  };

  return {
    acceptingStreams,
    streamBatchBuffers,
    thoughtBatchBuffers,
    appendStream,
    appendThought,
    flushStreamBatch,
    resetStreamState,
    dropStream,
    finalizeSessionStream,
    normalizeNewlines,
    normalizeToolLocations,
    scheduleSessionEventFlush,
    get pendingSessionEvents() { return pendingSessionEvents; },
    set pendingSessionEvents(v: string[]) { pendingSessionEvents = v; },
    get sessionEventFlushTimer() { return sessionEventFlushTimer; },
    set sessionEventFlushTimer(v: number | null) { sessionEventFlushTimer = v; },
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
