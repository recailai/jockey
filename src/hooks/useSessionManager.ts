import { createEffect, createMemo, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type { AppMessage, AppSession } from "../components/types";
import { now } from "../components/types";
import { MAX_MESSAGES } from "../lib/sessionHelpers";
import { appSessionApi } from "../lib/tauriApi";

export function useSessionManager() {
  const [sessions, setSessions] = createStore<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  const activeSession = createMemo(() => sessions.find((s) => s.id === activeSessionId()) ?? null);

  const updateSession = (id: string, patch: Partial<AppSession>) => {
    setSessions((s) => s.id === id, produce((s) => Object.assign(s, patch)));
  };

  const persistSessionPatch = (id: string, patch: Partial<AppSession>) => {
    const update: { title?: string; activeRole?: string; runtimeKind?: string | null | undefined } = {};
    if (typeof patch.title === "string") update.title = patch.title;
    if (typeof patch.activeRole === "string") update.activeRole = patch.activeRole;
    if ("runtimeKind" in patch) update.runtimeKind = patch.runtimeKind ?? null;
    if (Object.keys(update).length === 0) return;
    void appSessionApi.update(id, update).catch(() => { });
  };

  const patchActiveSession = (patch: Partial<AppSession>) => {
    const id = activeSessionId();
    if (!id) return;
    updateSession(id, patch);
    persistSessionPatch(id, patch);
  };

  let scrollRaf: number | null = null;
  const listRefMap = new Map<string, HTMLElement>();

  const scheduleScrollToBottom = () => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const id = activeSessionId();
      const el = id ? listRefMap.get(id) : null;
      if (el) el.scrollTop = el.scrollHeight;
    });
  };

  createEffect(() => {
    if (!activeSessionId()) return;
    const session = activeSession();
    void session?.messages.length;
    void session?.streamingMessage?.text.length;
    void session?.streamSegments.length;
    scheduleScrollToBottom();
  });

  const persistMessage = (sessionId: string, message: AppMessage) => {
    if (!sessionId || message.roleName === "event") return;
    void appSessionApi.appendMessage(sessionId, message.roleName, message.text).catch(() => {});
  };

  const appendMessageToSession = (sessionId: string, message: AppMessage) => {
    const idx = sessions.findIndex((s) => s.id === sessionId);
    if (idx === -1) return;
    setSessions(idx, "messages", produce((msgs: AppMessage[]) => {
      if (msgs.length >= MAX_MESSAGES) msgs.splice(0, msgs.length - MAX_MESSAGES + 1);
      msgs.push(message);
    }));
    scheduleScrollToBottom();
    persistMessage(sessionId, message);
  };

  const appendMessage = (message: AppMessage) => {
    const id = activeSessionId();
    if (!id) return;
    appendMessageToSession(id, message);
  };

  const pushMessage = (roleName: string, text: string) => {
    appendMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName, text, at: now() });
  };

  return {
    sessions,
    setSessions,
    activeSessionId,
    setActiveSessionId,
    activeSession,
    updateSession,
    patchActiveSession,
    persistSessionPatch,
    persistMessage,
    appendMessageToSession,
    appendMessage,
    pushMessage,
    listRefMap,
    scheduleScrollToBottom,
  };
}

export type SessionManager = ReturnType<typeof useSessionManager>;
