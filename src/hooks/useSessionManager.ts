import { createEffect, createMemo, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type { AppMessage, AppSession } from "../components/types";
import { now } from "../components/types";
import { MAX_MESSAGES } from "../lib/sessionHelpers";
import { appSessionApi } from "../lib/tauriApi";

export function useSessionManager() {
  const [sessions, setSessions] = createStore<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  // O(1) id → index lookup. Rebuilt whenever sessions array structure changes.
  const sessionIndexMap = new Map<string, number>();

  const rebuildIndexMap = () => {
    sessionIndexMap.clear();
    for (let i = 0; i < sessions.length; i++) {
      sessionIndexMap.set(sessions[i].id, i);
    }
  };

  // Keep index map in sync with sessions array.
  // Runs synchronously after any setSessions call that changes ids/order.
  createEffect(() => {
    // Track all ids to detect adds/removes/reorders
    void sessions.map((s) => s.id).join(",");
    rebuildIndexMap();
  });

  const activeSession = createMemo(() => {
    const id = activeSessionId();
    if (!id) return null;
    const idx = sessionIndexMap.get(id);
    return idx !== undefined ? sessions[idx] : null;
  });

  // O(1) by-id update — uses index map
  const updateSession = (id: string, patch: Partial<AppSession>) => {
    const idx = sessionIndexMap.get(id);
    if (idx === undefined) return;
    setSessions(idx, produce((s) => Object.assign(s, patch)));
  };

  // O(1) arbitrary mutation via produce — for complex state transforms
  const mutateSession = (id: string, recipe: (s: AppSession) => void) => {
    const idx = sessionIndexMap.get(id);
    if (idx === undefined) return;
    setSessions(idx, produce(recipe));
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

  type ScrollContainer = {
    el: HTMLElement;
    handler: () => void;
    scrolledUp: boolean;
    resizeObserver: ResizeObserver | null;
  };
  let scrollRaf: number | null = null;
  const scrollContainers = new Map<string, ScrollContainer>();

  const BOTTOM_THRESHOLD = 60;

  const scheduleScrollToBottom = (force = false) => {
    if (scrollRaf !== null) return;
    scrollRaf = window.requestAnimationFrame(() => {
      scrollRaf = null;
      const id = activeSessionId();
      const sc = id ? scrollContainers.get(id) : null;
      if (!sc) return;
      if (!force && sc.scrolledUp) return;
      sc.el.scrollTop = sc.el.scrollHeight;
    });
  };

  const onListMounted = (id: string, el: HTMLElement) => {
    const old = scrollContainers.get(id);
    if (old) {
      old.el.removeEventListener("scroll", old.handler);
      old.resizeObserver?.disconnect();
    }
    let scrollRafId: number | null = null;
    const sc: ScrollContainer = { el, handler: () => {}, scrolledUp: false, resizeObserver: null };
    const handler = () => {
      if (scrollRafId !== null) return;
      scrollRafId = window.requestAnimationFrame(() => {
        scrollRafId = null;
        const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
        sc.scrolledUp = distFromBottom > BOTTOM_THRESHOLD;
      });
    };
    sc.handler = handler;
    el.addEventListener("scroll", handler, { passive: true });
    sc.resizeObserver = new ResizeObserver(() => {
      if (!sc.scrolledUp) scheduleScrollToBottom(true);
    });
    sc.resizeObserver.observe(el);
    scrollContainers.set(id, sc);
    scheduleScrollToBottom(true);
  };

  const onListUnmounted = (id: string) => {
    const sc = scrollContainers.get(id);
    if (sc) {
      sc.el.removeEventListener("scroll", sc.handler);
      sc.resizeObserver?.disconnect();
    }
    scrollContainers.delete(id);
  };

  // Force scroll to bottom on session switch
  createEffect(() => {
    const id = activeSessionId();
    if (!id) return;
    scheduleScrollToBottom(true);
  });

  // Non-forced scroll on content updates within the current session
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

  // O(1) message append — uses index map
  const appendMessageToSession = (sessionId: string, message: AppMessage) => {
    const idx = sessionIndexMap.get(sessionId);
    if (idx === undefined) return;
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

  // O(1) session lookup for consumers outside this hook
  const getSessionIndex = (id: string): number => sessionIndexMap.get(id) ?? -1;

  return {
    sessions,
    setSessions,
    activeSessionId,
    setActiveSessionId,
    activeSession,
    updateSession,
    mutateSession,
    patchActiveSession,
    persistSessionPatch,
    persistMessage,
    appendMessageToSession,
    appendMessage,
    pushMessage,
    scheduleScrollToBottom,
    onListMounted,
    onListUnmounted,
    getSessionIndex,
  };
}

export type SessionManager = ReturnType<typeof useSessionManager>;
