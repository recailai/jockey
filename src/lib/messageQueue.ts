import type { AppSession } from "../components/types";

/** Read-only view of queued inputs for a given session id. */
export const queuedInputsFor = (
  sessions: readonly AppSession[],
  getSessionIndex: (id: string) => number,
  sid: string | null,
): readonly string[] => {
  if (!sid) return [];
  const idx = getSessionIndex(sid);
  return idx !== -1 ? (sessions[idx]?.queuedMessages ?? []) : [];
};

/** Merge queued inputs into a single newline-joined prompt.
 *  Returns empty string when no non-blank entries remain. */
export const mergeQueuedInputs = (items: readonly string[]): string =>
  items.map((q) => q.trim()).filter(Boolean).join("\n");

export type DequeueResult = { merged: string; count: number };

/** Decide what to send next from a session's queue. Caller is responsible for
 *  actually clearing the queue in the store; this is a pure projection. */
export const projectNextDequeue = (queue: readonly string[]): DequeueResult => ({
  merged: mergeQueuedInputs(queue),
  count: queue.length,
});
