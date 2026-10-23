import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { ArrowLeft, Square, X } from "lucide-solid";
import MessageWindow from "../components/MessageWindow";
import ChatInput from "../components/ChatInput";
import ProjectModal from "../components/ProjectModal";
import ImportSessionsModal from "../components/ImportSessionsModal";
import type { RichNode } from "../components/RichInput";
import { getPlainText, isImageNode } from "../components/RichInput";
import { DEFAULT_ROLE_ALIAS, now } from "../components/types";
import type {
  InputDeliveryCapabilities,
  QueuedItem,
  RuntimeCapabilities,
  SessionSummary,
} from "../components/types";
import { Badge, IconButton, Pill } from "../components/ui";
import { useSessionManager } from "../hooks/useSessionManager";
import { useStreamEngine } from "../hooks/useStreamEngine";
import { useAgentContext } from "../hooks/useAgentContext";
import { useAcpEventListeners } from "../hooks/useAcpEventListeners";
import { useMessageSend } from "../hooks/useMessageSend";
import { useProjects } from "../hooks/useProjects";
import { adoptLiveTurn, registerEngineEvents } from "../lib/engineEvents";
import { hydratePendingPrompts } from "../lib/pendingPrompts";
import { createSessionEventBuffer } from "../lib/sessionEventBuffer";
import { sessionFromRaw, uniqueName } from "../lib/sessionHelpers";
import {
  appSessionApi,
  engineApi,
  type InboxMessage,
  type Project,
  type RawSession,
} from "../lib/tauriApi";
import { call, listen, type UnlistenFn } from "../lib/transport";
import { useToast } from "../lib/useToast";
import SessionList from "./SessionList";
import ControlCenter from "../components/ControlCenter";
import {
  ClosedSessionsDialog,
  ConfirmDialog,
  DirectoryBrowserDialog,
  RolePickerDialog,
  TokenGate,
} from "./dialogs";
import type { HttpTransport } from "./httpTransport";

const SUMMARY_REFRESH_MS = 250;

const queuedFromInbox = (items: InboxMessage[]): QueuedItem[] =>
  items.map((item) => ({
    id: item.id,
    clientId: item.id,
    text: item.text,
    attachments: item.attachments ?? [],
    roleName: item.roleName,
    delivery: item.delivery === "nextStep" ? "nextStep" : "nextTurn",
    createdAt: item.createdAt,
    status: "queued",
  }));

export default function CompanionApp(props: { transport: HttpTransport }) {
  const { toasts, showToast } = useToast();
  const [authed, setAuthed] = createSignal(props.transport.hasToken());
  props.transport.onUnauthorized(() => setAuthed(false));

  return (
    <>
      <Show
        when={authed()}
        fallback={<TokenGate onSubmit={(token) => { props.transport.setToken(token); setAuthed(true); }} />}
      >
        <Shell transport={props.transport} showToast={showToast} />
      </Show>
      <div class="jockey-toast-stack fixed bottom-4 left-4 right-4 flex flex-col gap-2 pointer-events-none">
        <For each={toasts()}>
          {(t) => (
            <div class="jockey-toast pointer-events-auto" classList={{ "is-info": t.severity === "info", "is-danger": t.severity !== "info" }}>
              {t.message}
            </div>
          )}
        </For>
      </div>
    </>
  );
}

function Shell(props: { transport: HttpTransport; showToast: (m: string, s?: "error" | "info") => void }) {
  const { showToast } = props;

  const sessionManager = useSessionManager(showToast);
  const {
    sessions, setSessions,
    activeSessionId, setActiveSessionId,
    activeSession, updateSession, mutateSession, patchActiveSession,
    appendMessageToSession, pushMessage, scheduleScrollToBottom,
    getSessionIndex,
  } = sessionManager;

  const streamEngine = useStreamEngine(sessionManager);
  const { acceptingStreams, appendStream, appendThought, dropStream, normalizeToolLocations } = streamEngine;
  const agentContext = useAgentContext(sessionManager, showToast);
  const { roles, assistants, normalizeRuntimeKey, commandCacheKey, activeBackendRole, refreshAssistants, refreshRoles, resetActiveAgentContext, reconnectActiveAgent, parseAgentCommands } = agentContext;
  const { registerAcpEventListeners, clearSessionStream } = useAcpEventListeners();
  const { projects, createProject, refreshProjects } = useProjects(showToast);
  const { sendRaw, cancelCurrentRun, sendQueuedNow } = useMessageSend({
    sessionManager,
    closeMentionMenu: () => {},
    closeSlashMenu: () => {},
    showToast,
  });
  const sessionEventBuffer = createSessionEventBuffer({
    sessions,
    setSessions,
    getSessionIndex,
    pushMessage,
  });

  const [summaries, setSummaries] = createSignal<SessionSummary[]>([]);
  const [listState, setListState] = createSignal<"loading" | "ready" | "error">("loading");
  const [listError, setListError] = createSignal<string | null>(null);
  const [closedFor, setClosedFor] = createSignal<Project | null>(null);
  const [importFor, setImportFor] = createSignal<Project | null>(null);
  const [importAll, setImportAll] = createSignal(false);
  const [showProjectModal, setShowProjectModal] = createSignal(false);
  const [browseOpen, setBrowseOpen] = createSignal(false);
  const [roleOpen, setRoleOpen] = createSignal(false);
  const [closeOpen, setCloseOpen] = createSignal(false);
  const [browseSessions, setBrowseSessions] = createSignal(false);
  const drafts = new Map<string, RichNode[]>();
  const [richNodes, setRichNodes] = createSignal<RichNode[]>([]);
  createEffect(on(activeSessionId, (sid, previous) => {
    if (previous) drafts.set(previous, richNodes());
    setRichNodes(sid ? drafts.get(sid) ?? [] : []);
  }));
  let browseResolver: ((path: string | null) => void) | null = null;

  const refreshSummaries = async () => {
    try {
      setSummaries(await call<SessionSummary[]>("gateway_list_sessions"));
      setListState("ready");
      setListError(null);
    } catch (e) {
      setListError(String(e));
      setListState("error");
    }
  };
  let summaryTimer: number | undefined;
  const scheduleSummaries = () => {
    window.clearTimeout(summaryTimer);
    summaryTimer = window.setTimeout(() => void refreshSummaries(), SUMMARY_REFRESH_MS);
  };

  const refreshQueue = async (sid: string) => {
    try {
      const items = await appSessionApi.listInbox(sid);
      const idx = getSessionIndex(sid);
      if (idx !== -1) setSessions(idx, "queuedItems", queuedFromInbox(items));
    } catch { /* queue chips catch up on the next queue/changed */ }
  };

  const hydratePending = () => {
    void hydratePendingPrompts(
      { getSessionIndex, mutateSession },
      sessions.map((s) => s.id),
    ).catch(() => {});
  };

  const loadSession = async (id: string) => {
    const streamToken = acceptingStreams.get(id);
    const [raw] = await appSessionApi.getMany([id]);
    if (!raw) throw new Error("session not found");
    const session = sessionFromRaw(raw);
    if (getSessionIndex(id) === -1) setSessions((prev) => [...prev, session]);
    else {
      sessionManager.adoptMessages(id, session.messages);
      updateSession(id, {
        title: session.title,
        activeRole: session.activeRole,
        runtimeKind: session.runtimeKind,
        runtimeProfileId: session.runtimeProfileId,
        cwd: session.cwd,
      });
    }
    const running = (raw as RawSession & { running?: boolean }).running;
    if (running === false && acceptingStreams.get(id) === streamToken) {
      acceptingStreams.delete(id);
      clearSessionStream(id);
      updateSession(id, { submitting: false, turnPhase: "idle", status: "idle" });
    }
    await refreshQueue(id);
  };

  const openSession = async (id: string) => {
    try {
      await loadSession(id);
      window.history.pushState({ chat: id }, "");
      setActiveSessionId(id);
    } catch (e) {
      showToast(`Could not open session: ${String(e)}`);
    }
  };

  const closeChat = () => {
    setBrowseSessions(false);
    if (window.history.state?.chat) window.history.back();
    else setActiveSessionId(null);
  };

  const dropSessionLocally = (id: string) => {
    acceptingStreams.delete(id);
    if (activeSessionId() === id) setActiveSessionId(null);
    setSessions((prev) => prev.filter((s) => s.id !== id));
    scheduleSummaries();
  };

  const resync = () => {
    void refreshSummaries();
    void refreshProjects();
    hydratePending();
    const sid = activeSessionId();
    if (sid) {
      void loadSession(sid).catch(() => {});
      void appSessionApi.liveTurn(sid).then((live) => adoptLiveTurn({ getSessionIndex, adoptMessages: sessionManager.adoptMessages, updateSession, acceptingStreams }, sid, live)).catch(() => {});
    }
  };

  createEffect(on(activeSessionId, (sid) => {
    props.transport.setFocus(sid);
    if (!sid) return;
    void appSessionApi.watch(sid)
      .then((live) => adoptLiveTurn({ getSessionIndex, adoptMessages: sessionManager.adoptMessages, updateSession, acceptingStreams }, sid, live))
      .catch(() => {});
    void refreshQueue(sid);
    hydratePending();
  }));

  onMount(() => {
    const handlers: UnlistenFn[] = [];
    let disposed = false;
    const track = (p: Promise<UnlistenFn>) => void p.then((h) => (disposed ? h() : handlers.push(h)));

    const onPop = () => {
      const sid = window.history.state?.chat;
      setActiveSessionId(typeof sid === "string" && getSessionIndex(sid) !== -1 ? sid : null);
    };
    window.addEventListener("popstate", onPop);

    void refreshProjects();
    void refreshSummaries();
    void refreshAssistants();
    void refreshRoles();
    hydratePending();

    track(registerAcpEventListeners({
      acceptingStreams,
      sessions,
      getSessionIndex,
      appendStream,
      pushMessageToSession: (sid, role, text) =>
        appendMessageToSession(sid, { id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: role, text, at: now() }),
      pushMessage,
      onSessionDeltaLine: (sid, line, roleName) => sessionEventBuffer.push(sid, line, roleName),
      updateSession,
      mutateSession,
      appendThought,
      flushStream: streamEngine.flushStream,
      normalizeToolLocations,
      parseAgentCommands,
      normalizeRuntimeKey,
      roles,
      commandCacheKey,
      scheduleScrollToBottom,
    }).then((hs) => () => hs.forEach((h) => h())));

    track(registerEngineEvents({
      sessions,
      getSessionIndex,
      updateSession,
      setQueuedItems: (sid, items) => {
        const idx = getSessionIndex(sid);
        if (idx !== -1) setSessions(idx, "queuedItems", items);
      },
      appendSessions: (fresh) => setSessions((prev) => [...prev, ...fresh]),
      removeSession: dropSessionLocally,
      adoptMessages: sessionManager.adoptMessages,
      appendMessageLocal: sessionManager.appendMessageLocal,
      settleSessionStream: streamEngine.settleSessionStream,
      acceptingStreams,
      nextRunToken: agentContext.bumpRunToken,
      clearSessionStream,
      refreshProjects,
      showToast,
    }));

    for (const event of ["turn/started", "turn/finished", "session/changed", "project/changed", "message/added"]) {
      track(listen(event, scheduleSummaries));
    }
    track(listen("permission_update", hydratePending));
    const offResync = props.transport.onResync(resync);

    const onVisible = () => { if (document.visibilityState === "visible") resync(); };
    document.addEventListener("visibilitychange", onVisible);

    onCleanup(() => {
      disposed = true;
      handlers.forEach((h) => h());
      offResync();
      dropStream();
      window.clearTimeout(summaryTimer);
      window.removeEventListener("popstate", onPop);
      document.removeEventListener("visibilitychange", onVisible);
    });
  });

  const activeAssistant = createMemo(() => {
    const session = activeSession();
    return assistants().find((a) =>
      (session?.runtimeProfileId && a.profileId === session.runtimeProfileId) ||
      (!session?.runtimeProfileId && a.key === session?.runtimeKind),
    );
  });
  const runtimeCapabilities = () => activeAssistant()?.capabilities as RuntimeCapabilities | undefined;
  const inputDelivery = (): InputDeliveryCapabilities | undefined => activeAssistant()?.inputDelivery;

  const newSession = async (projectId: string) => {
    try {
      const titles = summaries().filter((s) => s.projectId === projectId).map((s) => s.title);
      const created = await appSessionApi.create(uniqueName("Session_1", titles), projectId);
      await openSession(created.id);
      void refreshSummaries();
    } catch (e) {
      showToast(`Could not create session: ${String(e)}`);
    }
  };

  const openRestored = (raw: RawSession) => {
    setClosedFor(null);
    void refreshSummaries();
    void openSession(raw.id);
  };

  const noTimer = { current: null as number | null };
  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const nodes = richNodes();
    const sid = activeSessionId();
    const text = getPlainText(nodes).trim();
    const attachments = nodes.filter(isImageNode).map((n) => n.img);
    if (!text && attachments.length === 0) return;
    setRichNodes([]);
    const sent = await sendRaw(text, false, sid, attachments.length ? attachments : undefined);
    if (!sent && sid) {
      if (activeSessionId() === sid && richNodes().length === 0) setRichNodes(nodes);
      else if (activeSessionId() !== sid && !drafts.get(sid)?.length) drafts.set(sid, nodes);
    }
  };

  const queuedCount = () => activeSession()?.queuedItems.filter((i) => i.status === "queued").length ?? 0;
  const projectName = () => projects().find((p) => p.id === activeSession()?.projectId)?.name ?? "No project";

  const closeActiveSession = async () => {
    const sid = activeSessionId();
    if (!sid) return;
    try {
      await appSessionApi.remove(sid);
      dropSessionLocally(sid);
    } catch (e) {
      showToast(`Could not close session: ${String(e)}`);
    }
  };

  const pickRole = async (roleName: string) => {
    const sid = activeSessionId();
    if (!sid) return;
    try {
      await appSessionApi.update(sid, { activeRole: roleName });
      await loadSession(sid);
    } catch (e) {
      showToast(`Could not switch persona: ${String(e)}`);
      throw e;
    }
  };

  const handlePasteImage = (items: DataTransferItemList) => {
    if (runtimeCapabilities()?.attachments === false) {
      showToast("This agent does not support image attachments.");
      return;
    }
    const sid = activeSessionId();
    for (const item of Array.from(items)) {
      if (item.kind !== "file" || !item.type.startsWith("image/")) continue;
      const file = item.getAsFile();
      if (!file) continue;
      const reader = new FileReader();
      reader.onload = () => {
        if (activeSessionId() !== sid || typeof reader.result !== "string") return;
        const data = reader.result.split(",")[1];
        if (!data) return;
        setRichNodes((prev) => [...prev, { kind: "image", index: Math.max(-1, ...prev.filter(isImageNode).map((n) => n.index)) + 1, img: { data, mimeType: file.type } }]);
      };
      reader.onerror = () => showToast("Could not read image attachment.");
      reader.readAsDataURL(file);
    }
  };

  return (
    <div class="companion-root">
      <Show
        when={activeSession()}
        fallback={
          <Show when={browseSessions()} fallback={
            <ControlCenter
              projects={projects}
              onOpenChat={(id) => void openSession(id)}
              onSend={async (id, text) => { await loadSession(id); return sendRaw(text, false, id); }}
              onStop={(id) => engineApi.cancel(id)}
              onBrowseSessions={() => setBrowseSessions(true)}
            />
          }>
          <SessionList
            onControlCenter={() => setBrowseSessions(false)}
            projects={projects}
            sessions={summaries}
            state={listState}
            error={listError}
            onRefresh={resync}
            onOpen={(id) => void openSession(id)}
            onNewSession={(pid) => void newSession(pid)}
            onShowClosed={setClosedFor}
            onImport={setImportFor}
            onImportAll={() => setImportAll(true)}
            onAddProject={() => setShowProjectModal(true)}
          />
          </Show>
        }
      >
        {(session) => (
          <>
            <header class="companion-header">
              <IconButton title="Sessions" aria-label="Back to sessions" onClick={closeChat}>
                <ArrowLeft size={18} />
              </IconButton>
              <div class="min-w-0 flex-1">
                <div class="truncate text-sm font-semibold">{session().title}</div>
                <div class="flex items-center gap-1.5 truncate text-[11px] theme-muted">
                  <span class="truncate">{projectName()}</span>
                  <Show when={session().externalActive}><Badge tone="info">terminal</Badge></Show>
                </div>
              </div>
              <Pill onClick={() => setRoleOpen(true)} title="Choose agent & persona">{activeAssistant()?.label ?? session().runtimeKind ?? "Agent"} · @{session().activeRole || DEFAULT_ROLE_ALIAS}</Pill>
              <Show when={session().submitting}>
                <IconButton title="Stop" aria-label="Stop" onClick={() => void cancelCurrentRun()}>
                  <Square size={16} />
                </IconButton>
              </Show>
              <IconButton title="Close session" aria-label="Close session" onClick={() => setCloseOpen(true)}>
                <X size={18} />
              </IconButton>
            </header>
            <div class="companion-chat-body">
              <MessageWindow
                presentation="responses"
                activeSessionId={activeSessionId}
                activeSession={activeSession}
                activeBackendRole={activeBackendRole}
                runtimeCapabilities={runtimeCapabilities}
                inputDelivery={inputDelivery}
                patchActiveSession={patchActiveSession}
                onRemoveQueuedMessage={(itemId) => {
                  void engineApi.removeQueued(session().id, itemId).catch((e: unknown) => showToast(`Could not remove queued message: ${String(e)}`));
                }}
                onSendQueuedNow={() => void sendQueuedNow()}
                onResetAgentContext={resetActiveAgentContext}
                onReconnectAgent={reconnectActiveAgent}
                onListMounted={sessionManager.onListMounted}
                onListUnmounted={sessionManager.onListUnmounted}
              />
              <ChatInput
                layout="active"
                placeholder={`Message ${session().activeRole || DEFAULT_ROLE_ALIAS}…`}
                richNodes={richNodes}
                setRichNodes={setRichNodes}
                activeRole={() => session().activeRole ?? DEFAULT_ROLE_ALIAS}
                submitting={() => session().submitting}
                queuedCount={queuedCount}
                onResetRole={() => {}}
                isCustomRole={() => true}
                onSubmit={handleSend}
                onInputKeyDown={() => {}}
                refreshInputCompletions={() => {}}
                mentionOpen={() => false}
                mentionItems={() => []}
                mentionActiveIndex={() => 0}
                slashOpen={() => false}
                slashItems={() => []}
                slashActiveIndex={() => 0}
                applyMentionCandidate={() => {}}
                applySlashCandidate={() => {}}
                closeMentionMenu={() => {}}
                closeSlashMenu={() => {}}
                richInputRef={() => {}}
                mentionCloseTimerRef={noTimer}
                mentionDebounceTimerRef={noTimer}
                hasImages={() => richNodes().some(isImageNode)}
                onPasteImage={handlePasteImage}
                onRemoveImage={(index) => setRichNodes((prev) => prev.filter((n) => !isImageNode(n) || n.index !== index))}
              />
            </div>
          </>
        )}
      </Show>

      <RolePickerDialog
        open={roleOpen()}
        projectId={activeSession()?.projectId ?? null}
        activeRole={activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS}
        assistants={assistants()}
        runtimeKind={activeSession()?.runtimeKind ?? null}
        disabled={activeSession()?.submitting ?? false}
        onPickRuntime={async (agent) => {
          const sid = activeSessionId();
          if (!sid) throw new Error("Open a session first.");
          await appSessionApi.update(sid, { runtimeKind: agent.key, runtimeProfileId: agent.profileId });
          await loadSession(sid);
        }}
        onPick={pickRole}
        onClose={() => setRoleOpen(false)}
      />
      <ConfirmDialog
        open={closeOpen()}
        title="Close this session?"
        description="You can restore it later from the project's History."
        confirmText="Close session"
        onConfirm={() => void closeActiveSession()}
        onClose={() => setCloseOpen(false)}
      />
      <ClosedSessionsDialog project={closedFor()} onClose={() => setClosedFor(null)} onRestored={openRestored} />
      <ImportSessionsModal
        open={importFor() !== null || importAll()}
        project={importFor()}
        initialScope={importAll() ? "all" : "project"}
        onClose={() => {
          setImportFor(null);
          setImportAll(false);
        }}
        onImported={(raw) => {
          setImportFor(null);
          setImportAll(false);
          resync();
          if (raw[0]) void openSession(raw[0].id);
        }}
        onOpenExisting={(id) => void openSession(id)}
      />
      <ProjectModal
        open={showProjectModal()}
        onClose={() => setShowProjectModal(false)}
        pickDirectory={() =>
          new Promise<string | null>((resolve) => {
            browseResolver = resolve;
            setBrowseOpen(true);
          })
        }
        onCreateProject={async (name, rootPath) => {
          await createProject(name, rootPath);
          void refreshSummaries();
        }}
      />
      <DirectoryBrowserDialog
        open={browseOpen()}
        onPick={(path) => { browseResolver?.(path); browseResolver = null; }}
        onClose={() => { browseResolver?.(null); browseResolver = null; setBrowseOpen(false); }}
      />
    </div>
  );
}
