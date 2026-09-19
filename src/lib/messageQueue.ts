import type { AppSession, QueuedItem } from "../components/types";

export const queuedItemsFor = (
  sessions: readonly AppSession[],
  getSessionIndex: (id: string) => number,
  sid: string | null,
): readonly QueuedItem[] => {
  if (!sid) return [];
  const idx = getSessionIndex(sid);
  return idx !== -1 ? (sessions[idx]?.queuedItems ?? []) : [];
};

const mergeQueuedInputs = (items: readonly string[]): string =>
  items.map((q) => q.trim()).filter(Boolean).join("\n");

export type QueuedDequeueResult = {
  items: QueuedItem[];
  merged: string;
  attachments: Array<{ data: string; mimeType: string }>;
  roleName: string | null;
};

export const projectQueuedItemDequeue = (items: readonly QueuedItem[]): QueuedDequeueResult => {
  const first = items.find((item) => item.status === "queued");
  const roleName = first?.roleName ?? null;
  const selected: QueuedItem[] = [];
  if (first) {
    for (const item of items) {
      if (item.status !== "queued") continue;
      if ((item.roleName ?? null) !== roleName) break;
      selected.push(item);
    }
  }
  return {
    items: selected,
    merged: mergeQueuedInputs(selected.map((item) => item.text)),
    attachments: selected.flatMap((item) => item.attachments),
    roleName,
  };
};
