import { createEffect, createMemo, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type { AppMessage, AppSession } from "../components/types";
import { now } from "../components/types";
import { MAX_MESSAGES } from "../lib/sessionHelpers";
import { appSessionApi } from "../lib/tauriApi";

export function useSessionManager(
  showToast?: (message: string, severity?: "error" | "info") => void,
) {
  // A write that fails here means the UI and the database have diverged — the change looks
  // applied until the next load silently reverts it. Surface it rather than swallowing it.
  const reportPersistFailure = (what: string, e: unknown) => {
    showToast?.(`Could not save ${what}: ${String(e)}`, "error");
  };

  const [sessions, setSessions] = createStore<AppSession[]>([]);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);

  const getSessionIndex = (id: string): number => {
    for (let i = 0; i < sessions.length; i++) {
      if (sessions[i].id === id) return i;
    }
    return -1;
  };

  const activeSession = createMemo(() => {
    const id = activeSessionId();
    if (!id) return null;
    return sessions.find((s) => s.id === id) ?? null;
  });

  // By-id update
  const updateSession = (id: string, patch: Partial<AppSession>) => {
    const idx = getSessionIndex(id);
    if (idx === -1) return;
    setSessions(idx, produce((s) => Object.assign(s, patch)));
  };

  // Arbitrary mutation via produce
  const mutateSession = (id: string, recipe: (s: AppSession) => void) => {
    const idx = getSessionIndex(id);
    if (idx === -1) return;
    setSessions(idx, produce(recipe));
  };

  const persistSessionPatch = (id: string, patch: Partial<AppSession>) => {
    // Drafts have no DB row to update yet; their state is carried in-memory until
    // `ensureSessionPersisted` creates the row (with this same state) at send time.
    if (!sessions[getSessionIndex(id)]?.persisted) return;
    const update: {
      title?: string;
      activeRole?: string;
      runtimeKind?: string | null | undefined;
      runtimeProfileId?: string | null | undefined;
    } = {};
    if (typeof patch.title === "string") update.title = patch.title;
    if (typeof patch.activeRole === "string") update.activeRole = patch.activeRole;
    if ("runtimeKind" in patch) update.runtimeKind = patch.runtimeKind ?? null;
    if ("runtimeProfileId" in patch) update.runtimeProfileId = patch.runtimeProfileId ?? null;
    if (Object.keys(update).length === 0) return;
    void appSessionApi
      .update(id, update)
      .catch((e: unknown) => reportPersistFailure("this session's persona/engine", e));
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
    // A draft session has no `app_sessions` row yet (FK target for app_session_messages),
    // and this can fire for local notices before the first real send persists it.
    if (!sessions[getSessionIndex(sessionId)]?.persisted) return;
    // Tool calls/diffs/plans/images/thoughts only ever live in memory otherwise — persist
    // them as structured JSON so they survive a reload instead of collapsing to bare text.
    const hasStructured =
      (message.toolCalls?.length ?? 0) > 0 ||
      (message.segments?.some((s) => s.kind !== "text") ?? false) ||
      (message.images?.length ?? 0) > 0 ||
      !!message.thoughtText;
    if (hasStructured) {
      const payload = JSON.stringify({
        text: message.text,
        toolCalls: message.toolCalls,
        segments: message.segments,
        images: message.images,
        thoughtText: message.thoughtText,
      });
      void appSessionApi
        .appendMessage(sessionId, message.roleName, message.text, "json", payload)
        .catch((e: unknown) => reportPersistFailure("a message", e));
    } else {
      void appSessionApi
        .appendMessage(sessionId, message.roleName, message.text)
        .catch((e: unknown) => reportPersistFailure("a message", e));
    }
  };

  // In-flight creates for draft sessions, so a fast double-send (or a queued send racing a
  // fresh one) awaits the same create instead of issuing two and risking a duplicate row.
  const persistInFlight = new Map<string, Promise<void>>();

  /** Turn a draft session into a real one by creating its `app_sessions` row, reusing the
   *  draft's own client-generated id as the DB primary key. No-op if already persisted. */
  const ensureSessionPersisted = (id: string): Promise<void> => {
    const idx = getSessionIndex(id);
    if (idx === -1) return Promise.resolve();
    const session = sessions[idx];
    if (session.persisted) return Promise.resolve();
    const inFlight = persistInFlight.get(id);
    if (inFlight) return inFlight;
    const promise = (async () => {
      const created = await appSessionApi.create(
        session.title,
        session.projectId ?? undefined,
        session.runtimeKind ?? undefined,
        session.runtimeProfileId ?? undefined,
        session.id,
      );
      updateSession(id, { persisted: true, title: created.title || session.title });
    })();
    persistInFlight.set(id, promise);
    void promise.finally(() => persistInFlight.delete(id));
    return promise;
  };

  const appendMessageToSession = (sessionId: string, message: AppMessage) => {
    const idx = getSessionIndex(sessionId);
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
    mutateSession,
    patchActiveSession,
    persistSessionPatch,
    persistMessage,
    ensureSessionPersisted,
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
