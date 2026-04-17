import { now } from "../components/types";
import type { AppMessage, AppSession } from "../components/types";
import type { RunToken } from "./runToken";

/**
 * Aggregate that owns a single run's lifetime on one session's live stream.
 *
 * Each invariant is captured by construction:
 *  - `runToken` stamps every write the aggregate makes, so concurrent runs
 *    targeting the same session can't cross-contaminate (see bug: "UI doesn't
 *    update after cancel + queued send").
 *  - `originSessionId === null` collapses the aggregate to a no-op (used by
 *    flows that execute without a bound session, e.g. early bootstrap).
 */

export type StreamSessionDeps = {
  appendMessageToSession: (sid: string, msg: AppMessage) => void;
  patchSession: (sid: string, patch: Partial<AppSession>) => void;
  finalizeSessionStream: (
    sid: string,
    fallbackRoleName: string,
    finalReply?: string,
    expectedRunToken?: number,
  ) => void;
  resetStreamState: (sid?: string) => void;
  scheduleScrollToBottom: () => void;
  getSession: (sid: string) => AppSession | null;
  acceptingStreams: Map<string, number>;
};

export type StreamSession = {
  readonly runToken: RunToken;
  readonly sessionId: string | null;
  readonly roleLabel: string;
  /** Append an out-of-band message (event, error, non-command result) to the session. */
  appendMessage: (msg: AppMessage) => void;
  /** Open the live stream: register token in acceptingStreams + create empty streamingMessage. */
  start: () => void;
  /** Finalize streamingMessage into a persistent message, token-guarded. */
  complete: (finalReply?: string) => void;
  /** Drop streamingMessage if still owned by this run (no-op if newer run took over). */
  drop: () => void;
};

export function createStreamSession(
  originSessionId: string | null,
  roleLabel: string,
  runToken: RunToken,
  deps: StreamSessionDeps,
): StreamSession {
  const appendMessage = (msg: AppMessage) => {
    if (!originSessionId) return;
    deps.appendMessageToSession(originSessionId, msg);
  };

  const start = () => {
    if (!originSessionId) return;
    deps.acceptingStreams.set(originSessionId, runToken);
    const row: AppMessage = {
      id: `stream-${now()}`,
      roleName: roleLabel,
      text: "",
      at: now(),
    };
    deps.patchSession(originSessionId, {
      streamingMessage: row,
      streamingRunToken: runToken,
      toolCalls: {},
      streamSegments: [],
      currentPlan: null,
      pendingPermission: null,
      thoughtText: "",
    });
    deps.scheduleScrollToBottom();
  };

  const complete = (finalReply?: string) => {
    if (!originSessionId) return;
    deps.finalizeSessionStream(originSessionId, roleLabel, finalReply, runToken);
  };

  const drop = () => {
    if (!originSessionId) return;
    const s = deps.getSession(originSessionId);
    // Only clear if the session's stream is still owned by this run.
    if (s && s.streamingRunToken !== null && s.streamingRunToken !== runToken) return;
    deps.patchSession(originSessionId, {
      streamingMessage: null,
      streamingRunToken: null,
      thoughtText: "",
    });
    deps.resetStreamState(originSessionId);
  };

  return {
    runToken,
    sessionId: originSessionId,
    roleLabel,
    appendMessage,
    start,
    complete,
    drop,
  };
}
