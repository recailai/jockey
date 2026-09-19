import { produce } from "solid-js/store";
import { now } from "../components/types";
import type { AppMessage, AppSession } from "../components/types";
import { MAX_MESSAGES } from "./sessionHelpers";

const FLUSH_DELAY_MS = 120;

type BufferEntry = { sid: string; line: string; roleName?: string };

export type SessionEventBufferDeps = {
  sessions: AppSession[];
  setSessions: (idx: number, path: "messages", producer: (msgs: AppMessage[]) => void) => void;
  getSessionIndex: (id: string) => number;
  pushMessage: (role: string, text: string) => void;
};

export type SessionEventBuffer = {
  push: (sid: string, line: string, roleName?: string) => void;
};

/** Coalesces `session/update` lines over a 120ms window and appends them as a
 *  single event message per session. Keeps streaming UI quiet when agents emit
 *  many small status lines. */
export function createSessionEventBuffer(deps: SessionEventBufferDeps): SessionEventBuffer {
  let pending: BufferEntry[] = [];
  let timer: number | null = null;

  const flush = () => {
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    if (pending.length === 0) return;

    const byGroup = new Map<string, { sid: string; roleName?: string; lines: string[] }>();
    for (const { sid, line, roleName } of pending) {
      const key = `${sid || "__active__"}\u0000${roleName ?? "event"}`;
      const group = byGroup.get(key) ?? { sid, roleName, lines: [] };
      group.lines.push(line);
      byGroup.set(key, group);
    }
    pending = [];

    for (const group of byGroup.values()) {
      const text = group.lines.join("\n");
      if (!group.sid) {
        deps.pushMessage(group.roleName ?? "event", text);
        continue;
      }
      const idx = deps.getSessionIndex(group.sid);
      if (idx === -1) {
        deps.pushMessage(group.roleName ?? "event", text);
        continue;
      }
      const msg: AppMessage = {
        id: `${now()}-${Math.random().toString(36).slice(2)}`,
        roleName: group.roleName ?? "event",
        text,
        at: now(),
      };
      deps.setSessions(idx, "messages", produce((msgs: AppMessage[]) => {
        if (msgs.length >= MAX_MESSAGES) msgs.splice(0, msgs.length - MAX_MESSAGES + 1);
        msgs.push(msg);
      }));
    }
  };

  const schedule = () => {
    if (timer !== null) return;
    timer = window.setTimeout(() => {
      timer = null;
      flush();
    }, FLUSH_DELAY_MS);
  };

  const push = (sid: string, line: string, roleName?: string) => {
    pending.push({ sid, line, roleName });
    schedule();
  };

  return { push };
}
