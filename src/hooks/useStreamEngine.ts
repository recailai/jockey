import { produce } from "solid-js/store";
import { now } from "../components/types";
import type { AppMessage } from "../components/types";
import { MAX_THOUGHT_CHARS, MAX_MESSAGES } from "../lib/sessionHelpers";
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
    getSessionIndex,
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
    // Collect all pending buffers keyed by session id
    const textChunks = new Map<string, string>();
    for (const [sid, buf] of streamBatchBuffers) {
      if (!buf) continue;
      textChunks.set(sid, buf);
      streamBatchBuffers.set(sid, "");
    }
    const thoughtChunks = new Map<string, string>();
    for (const [sid, buf] of thoughtBatchBuffers) {
      if (!buf) continue;
      thoughtChunks.set(sid, buf);
      thoughtBatchBuffers.set(sid, "");
    }

    // Merge text + thought updates into a single setSessions call per session
    const allSids = new Set([...textChunks.keys(), ...thoughtChunks.keys()]);
    for (const sid of allSids) {
      const textChunk = textChunks.get(sid);
      const thoughtChunk = thoughtChunks.get(sid);
      const idx = getSessionIndex(sid);
      if (idx === -1) continue;
      setSessions(
        idx,
        produce((s) => {
          if (textChunk && s.streamingMessage) {
            s.streamingMessage.text = (s.streamingMessage.text ?? "") + textChunk;
            const last = s.streamSegments[s.streamSegments.length - 1];
            if (last && last.kind === "text") {
              s.streamSegments[s.streamSegments.length - 1] = { kind: "text" as const, text: last.text + textChunk };
            } else {
              s.streamSegments.push({ kind: "text" as const, text: textChunk });
            }
          }
          if (thoughtChunk) {
            const next = `${s.thoughtText ?? ""}${thoughtChunk}`;
            s.thoughtText = next.length <= MAX_THOUGHT_CHARS ? next : next.slice(next.length - MAX_THOUGHT_CHARS);
          }
        }),
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
    if (sessionId) {
      streamBatchBuffers.delete(sessionId);
      thoughtBatchBuffers.delete(sessionId);
      acceptingStreams.delete(sessionId);
      const anyPending = [...streamBatchBuffers.values(), ...thoughtBatchBuffers.values()].some(v => v);
      if (!anyPending && streamBatchRaf !== null) {
        window.cancelAnimationFrame(streamBatchRaf);
        streamBatchRaf = null;
      }
    } else {
      if (streamBatchRaf !== null) {
        window.cancelAnimationFrame(streamBatchRaf);
        streamBatchRaf = null;
      }
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
    const idx = getSessionIndex(sessionId);
    const sess = idx !== -1 ? sessions[idx] : undefined;
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

  let pendingSessionEvents: Array<{ sid: string; line: string }> = [];
  let sessionEventFlushTimer: number | null = null;

  const scheduleSessionEventFlush = () => {
    if (sessionEventFlushTimer !== null) return;
    sessionEventFlushTimer = window.setTimeout(() => {
      sessionEventFlushTimer = null;
      if (pendingSessionEvents.length === 0) return;
      const bySid = new Map<string, string[]>();
      for (const { sid, line } of pendingSessionEvents) {
        const key = sid || "__active__";
        const arr = bySid.get(key) ?? [];
        arr.push(line);
        bySid.set(key, arr);
      }
      for (const [sid, lines] of bySid) {
        const text = lines.join("\n");
        if (sid === "__active__") {
          pushMessage("event", text);
        } else {
          const idx = getSessionIndex(sid);
          if (idx === -1) {
            pushMessage("event", text);
          } else {
            const msg = { id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: "event", text, at: now() };
            setSessions(idx, "messages", produce((msgs: AppMessage[]) => {
              if (msgs.length >= MAX_MESSAGES) msgs.splice(0, msgs.length - MAX_MESSAGES + 1);
              msgs.push(msg);
            }));
          }
        }
      }
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
    set pendingSessionEvents(v: Array<{ sid: string; line: string }>) { pendingSessionEvents = v; },
    get sessionEventFlushTimer() { return sessionEventFlushTimer; },
    set sessionEventFlushTimer(v: number | null) { sessionEventFlushTimer = v; },
  };
}

export type StreamEngine = ReturnType<typeof useStreamEngine>;
