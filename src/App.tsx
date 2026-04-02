import { For, Show, Suspense, createSignal, lazy, onCleanup, onMount } from "solid-js";

import SessionTabs from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import type { AppMessage, AppSession } from "./components/types";
import {
  now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS,
} from "./components/types";
import { type UiTheme, normalizeUiTheme, UI_THEME_KEY } from "./lib/theme";

import { useSessionManager } from "./hooks/useSessionManager";
import { useStreamEngine } from "./hooks/useStreamEngine";
import { useAgentContext } from "./hooks/useAgentContext";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useAcpEventListeners } from "./hooks/useAcpEventListeners";
import { useCompletions } from "./hooks/useCompletions";
import {
  uniqueName, isDefaultSessionTitle, makeDefaultSession, deriveSessionTitleFromMessage,
} from "./lib/sessionHelpers";
import { appSessionApi, assistantApi } from "./lib/tauriApi";
import { parseAgentControlCommand, resolveRoute, type ResolveRouteResult } from "./lib/chatPipeline";
import { mutateSessionWithProduce } from "./lib/acpEventBridge";

const ConfigDrawer = lazy(() => import("./components/ConfigDrawer"));
const ManagementPanel = lazy(() => import("./components/ManagementPanel"));

export default function App() {
  type Toast = { id: number; message: string; severity?: "error" | "info" };
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  let toastSeq = 0;
  const initialTheme = (): UiTheme => {
    try {
      const raw = window.localStorage.getItem(UI_THEME_KEY);
      const theme = normalizeUiTheme(raw);
      document.documentElement.setAttribute("data-theme", theme);
      return theme;
    } catch {
      document.documentElement.setAttribute("data-theme", "dark");
      return "dark";
    }
  };

  const [uiTheme, setUiTheme] = createSignal<UiTheme>(initialTheme());

  const showToast = (message: string, severity: Toast["severity"] = "error") => {
    const id = ++toastSeq;
    setToasts((ts) => [...ts, { id, message, severity }]);
    window.setTimeout(() => setToasts((ts) => ts.filter((t) => t.id !== id)), 4000);
  };

  const [showDrawer, setShowDrawer] = createSignal(false);
  const [showManagement, setShowManagement] = createSignal(false);
  const [managementInitialTab, setManagementInitialTab] = createSignal<"sessions" | "workflows" | "roles" | "mcp" | "skills">("sessions");
  const [managementInitialRole, setManagementInitialRole] = createSignal<string | undefined>(undefined);

  const [input, setInput] = createSignal("");
  let inputEl: HTMLInputElement | undefined;

  const sessionManager = useSessionManager();
  const {
    sessions, setSessions,
    activeSessionId, setActiveSessionId,
    activeSession,
    updateSession, patchActiveSession,
    appendMessageToSession, pushMessage,
    scheduleScrollToBottom,
    onListMounted, onListUnmounted,
  } = sessionManager;

  const streamEngine = useStreamEngine(sessionManager);
  const {
    acceptingStreams, streamBatchBuffers, thoughtBatchBuffers,
    appendStream, appendThought,
    resetStreamState, dropStream,
    finalizeSessionStream,
    normalizeToolLocations,
    scheduleSessionEventFlush,
  } = streamEngine;

  const agentContext = useAgentContext(sessionManager, streamEngine, showToast);
  const {
    roles, assistants, skills,
    normalizeRuntimeKey, commandCacheKey,
    bumpRunToken, getCanceledRunToken,
    isCustomRole, activeBackendRole,
    refreshRoles, refreshSkills,
    fetchConfigOptions, fetchModes,
    parseAgentCommands,
    fetchAndCacheAgentCommands,
    setPreferredAssistant, refreshAssistants,
    resetActiveAgentContext,
    reconnectActiveAgent,
    cancelCurrentRun: cancelCurrentRunBase,
  } = agentContext;

  const completions = useCompletions(
    agentContext,
    sessionManager,
    input,
    setInput,
    () => inputEl,
  );
  const {
    mentionOpen, mentionItems, mentionActiveIndex,
    slashOpen, slashItems, slashActiveIndex,
    mentionCloseTimerRef, mentionDebounceTimerRef,
    closeMentionMenu, closeSlashMenu,
    refreshInputCompletions,
    applyMentionCandidate, applySlashCandidate,
  } = completions;
  const { registerAcpEventListeners } = useAcpEventListeners();

  const { bootstrapApp } = useAppBootstrap({
    setSessions,
    setActiveSessionId: (id) => setActiveSessionId(id),
    assistants,
    refreshAssistants,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
    pushMessage,
    showToast,
  });

  const queuedInputsFor = (sid: string | null): string[] => {
    if (!sid) return [];
    const s = sessions.find((x) => x.id === sid);
    return s?.queuedMessages ?? [];
  };

  let inputHistory: string[] = [];
  let historyIndex = -1;
  let historySavedInput = "";
  const HISTORY_MAX = 200;

  const runNextQueued = (preferredSessionId?: string | null) => {
    const sid = preferredSessionId ?? activeSessionId();
    if (!sid) return;
    const s = sessions.find((x) => x.id === sid);
    if (s?.submitting) return;
    const queue = queuedInputsFor(sid);
    if (queue.length === 0) return;
    setSessions((ss) => ss.id === sid, "queuedMessages", []);
    const merged = queue.map((q) => q.trim()).filter(Boolean).join("\n");
    if (!merged) return;
    if (queue.length > 1) {
      appendMessageToSession(sid, { id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: "event", text: `queued messages merged: ${queue.length}`, at: now() });
    }
    void sendRaw(merged, false, sid);
  };

  const cancelCurrentRun = async () => {
    await cancelCurrentRunBase(runNextQueued);
  };

  const applyRouteState = (route: ResolveRouteResult) => {
    if (route.activateRole === DEFAULT_ROLE_ALIAS) {
      patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS });
      return;
    }
    if (route.activateRole) {
      patchActiveSession({ activeRole: route.activateRole, discoveredConfigOptions: [] });
    }
  };

  const patchSessionById = (sessionId: string | null, patch: Partial<AppSession>) => {
    if (!sessionId) return;
    updateSession(sessionId, patch);
  };

  const buildOriginStreamOps = (originSessionId: string | null, sendRoleLabel: string) => {
    const appendOriginMessage = (msg: AppMessage) => {
      if (!originSessionId) return;
      appendMessageToSession(originSessionId, msg);
    };

    const startOriginStream = () => {
      if (originSessionId) acceptingStreams.add(originSessionId);
      const id = `stream-${now()}`;
      const row: AppMessage = { id, roleName: sendRoleLabel, text: "", at: now() };
      patchSessionById(originSessionId, { streamingMessage: row, toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, thoughtText: "" });
      scheduleScrollToBottom();
      return row;
    };

    const completeOriginStream = (finalReply?: string) => {
      if (!originSessionId) return;
      finalizeSessionStream(originSessionId, sendRoleLabel, finalReply);
    };

    const dropOriginStream = () => {
      patchSessionById(originSessionId, { streamingMessage: null, thoughtText: "" });
      resetStreamState(originSessionId ?? undefined);
    };

    return {
      appendOriginMessage,
      startOriginStream,
      completeOriginStream,
      dropOriginStream,
    };
  };

  const prefetchRoleResources = (roleName: string) => {
    const targetRole = roles().find((r) => r.roleName === roleName);
    if (!targetRole) return;
    void fetchConfigOptions(targetRole.runtimeKind, targetRole.roleName).then((opts) => patchActiveSession({ discoveredConfigOptions: opts }));
    fetchAndCacheAgentCommands(targetRole.runtimeKind, targetRole.roleName);
  };

  const maybeAutoTitleSession = (sid: string, text: string) => {
    const sess = sessions.find((x) => x.id === sid);
    if (!sess) return;
    if (!isDefaultSessionTitle(sess.title)) return;
    if (sess.messages.filter((m) => m.roleName === "user").length !== 0) return;
    const existing = sessions.filter((x) => x.id !== sid).map((x) => x.title);
    const autoTitle = deriveSessionTitleFromMessage(text, existing);
    updateSession(sid, { title: autoTitle });
    void appSessionApi.update(sid, { title: autoTitle }).catch(() => {});
  };

  const runAgentControlCommand = async (
    text: string,
    isCommand: boolean,
    inRoleContext: boolean,
    originSessionId: string | null,
    patchOriginSession: (patch: Partial<AppSession>) => void,
  ): Promise<boolean> => {
    const cmd = parseAgentControlCommand(text, isCommand, inRoleContext);
    if (!cmd) return false;
    const role = activeBackendRole();
    try {
      if (cmd === "cancel") {
        if (originSessionId) await assistantApi.cancelSession(role, originSessionId);
        pushMessage("event", `cancelled ${role}`);
      } else {
        if (originSessionId) await assistantApi.setMode(role, cmd, originSessionId);
        pushMessage("event", `${role} mode → ${cmd}`);
      }
    } catch (e) {
      pushMessage("event", String(e));
    } finally {
      patchOriginSession({ submitting: false, status: "idle" });
      runNextQueued(originSessionId);
    }
    return true;
  };

  const sendRaw = async (text: string, silent = false, targetSessionId?: string | null) => {
    const runToken = bumpRunToken();
    const originSessionId = targetSessionId ?? activeSessionId();
    closeMentionMenu();
    closeSlashMenu();

    const patchOriginSession = (patch: Partial<AppSession>) => {
      patchSessionById(originSessionId, patch);
    };

    const s = sessions.find((x) => x.id === originSessionId) ?? activeSession();
    const sessionIsCustomRole = s ? s.activeRole !== DEFAULT_ROLE_ALIAS && s.activeRole !== DEFAULT_BACKEND_ROLE : false;
    const route = resolveRoute({
      text,
      activeRole: s?.activeRole ?? DEFAULT_ROLE_ALIAS,
      roleNames: roles().map((r) => r.roleName),
      isCustomRole: sessionIsCustomRole,
      defaultRoleAlias: DEFAULT_ROLE_ALIAS,
      defaultBackendRole: DEFAULT_BACKEND_ROLE,
    });
    if (route.error) {
      pushMessage("event", route.error);
      return;
    }
    if (originSessionId === activeSessionId()) {
      applyRouteState(route);
    }
    if (route.prefetchRole) {
      prefetchRoleResources(route.prefetchRole);
    }
    if (route.explicitRoleMention && !route.routedText) {
      return;
    }
    const sendRoleLabel = route.sendRoleLabel;
    const isCommand = route.isCommand;
    const inRoleContext = route.inRoleContext;
    const isAppCommand = route.isAppCommand;
    const routedText = route.routedText;

    if (!silent) {
      if (originSessionId) {
        appendMessageToSession(originSessionId, { id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: "user", text, at: now() });
        maybeAutoTitleSession(originSessionId, text);
      } else {
        pushMessage("user", text);
      }
    }
    patchOriginSession({ submitting: true, status: "running", agentState: undefined, thoughtText: "" });

    if (await runAgentControlCommand(text, isCommand, inRoleContext, originSessionId, patchOriginSession)) {
      return;
    }
    const { appendOriginMessage, startOriginStream, completeOriginStream, dropOriginStream } =
      buildOriginStreamOps(originSessionId, sendRoleLabel);

    let streamStarted = false;
    let finalStatus: "done" | "error" = "done";
    if ((!isAppCommand) && s?.runtimeKind) {
      startOriginStream();
      streamStarted = true;
    }

    try {
      const res = await assistantApi.chat({
        input: routedText,
        runtimeKind: s?.runtimeKind ?? null,
        appSessionId: originSessionId ?? null,
      });
      if (runToken <= getCanceledRunToken()) return;
      if (res.runtimeKind && originSessionId === activeSessionId()) setPreferredAssistant(res.runtimeKind);
      if (text.startsWith("/app_role")) void refreshRoles();

      if (!res.ok) {
        if (streamStarted) dropOriginStream();
        showToast(res.reply);
        appendOriginMessage({ id: `${now()}-err`, roleName: "event", text: res.reply, at: now() });
        finalStatus = "error";
        return;
      }

      if (streamStarted) {
        completeOriginStream(res.reply);
      } else {
        appendOriginMessage({ id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: sendRoleLabel, text: res.reply, at: now() });
      }

    } catch (e) {
      if (runToken <= getCanceledRunToken()) return;
      dropOriginStream();
      const errMsg = String(e);
      if (!errMsg.toLowerCase().includes("cancel")) showToast(errMsg);
      appendOriginMessage({ id: `${now()}-err`, roleName: "event", text: errMsg, at: now() });
      finalStatus = "error";
      return;
    } finally {
      if (originSessionId) acceptingStreams.delete(originSessionId);
      if (runToken <= getCanceledRunToken()) {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued(originSessionId);
        return;
      }
      patchOriginSession({ submitting: false, status: finalStatus });
      runNextQueued(originSessionId);
    }
  };

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const text = input().trim();
    if (!text) return;
    if (activeSession()?.submitting) {
      setInput("");
      const sid = activeSessionId();
      if (sid) {
        setSessions((s) => s.id === sid, "queuedMessages", (prev) => [...prev, text]);
        scheduleScrollToBottom();
      }
      return;
    }
    if (!activeSession()?.runtimeKind && !isCustomRole() && !text.startsWith("/app_")) {
      pushMessage("system", "Select an assistant or a role first.");
      return;
    }
    closeMentionMenu();
    closeSlashMenu();
    inputHistory.unshift(text);
    if (inputHistory.length > HISTORY_MAX) inputHistory.length = HISTORY_MAX;
    historyIndex = -1;
    historySavedInput = "";
    setInput("");
    await sendRaw(text);
  };

  const handleInputEvent = (el: HTMLInputElement) => {
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    setInput(value);
    historyIndex = -1;
    if (mentionDebounceTimerRef.current !== null) window.clearTimeout(mentionDebounceTimerRef.current);
    mentionDebounceTimerRef.current = window.setTimeout(() => {
      mentionDebounceTimerRef.current = null;
      refreshInputCompletions(value, caret);
    }, 90);
  };

  const handleInputKeyDownFinal = (e: KeyboardEvent) => {
    if (e.key === "Escape" && activeSession()?.submitting) {
      e.preventDefault();
      void cancelCurrentRun();
      return;
    }

    const slash = slashItems();
    if (slashOpen() && slash.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        // We need setSlashActiveIndex — expose it from completions
        completions._setSlashActiveIndex((i: number) => (i + 1) % slash.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        completions._setSlashActiveIndex((i: number) => (i - 1 + slash.length) % slash.length);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        applySlashCandidate(slash[slashActiveIndex()] ?? slash[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeSlashMenu();
        return;
      }
    }

    const items = mentionItems();
    if (mentionOpen() && items.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        completions._setMentionActiveIndex((i: number) => (i + 1) % items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        completions._setMentionActiveIndex((i: number) => (i - 1 + items.length) % items.length);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        applyMentionCandidate(items[mentionActiveIndex()] ?? items[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeMentionMenu();
        return;
      }
    }

    if (e.key === "ArrowUp" && inputHistory.length > 0) {
      e.preventDefault();
      if (historyIndex === -1) historySavedInput = input();
      if (historyIndex < inputHistory.length - 1) {
        historyIndex++;
        setInput(inputHistory[historyIndex]);
      }
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (historyIndex > 0) {
        historyIndex--;
        setInput(inputHistory[historyIndex]);
      } else if (historyIndex === 0) {
        historyIndex = -1;
        setInput(historySavedInput);
      }
      return;
    }
  };

  const newSession = () => {
    const availableAssistant = assistants().find((a) => a.available)?.key ?? null;
    const title = uniqueName("Session_1", sessions.map((s) => s.title));
    void appSessionApi.create(title).then((created) => {
      const s = makeDefaultSession(title);
      s.id = created.id;
      s.runtimeKind = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
      if (availableAssistant) {
        void appSessionApi.update(created.id, { runtimeKind: availableAssistant }).catch(() => { });
      }
    }).catch((e: unknown) => {
      showToast(`Failed to create session: ${String(e)}`);
      const s = makeDefaultSession(title);
      s.runtimeKind = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
    });
  };

  const closeSession = (id: string) => {
    streamBatchBuffers.delete(id);
    thoughtBatchBuffers.delete(id);
    acceptingStreams.delete(id);
    const remaining = sessions.filter((s) => s.id !== id);
    if (remaining.length === 0) { void appSessionApi.remove(id).catch(() => {}); return; }
    setSessions(remaining);
    if (activeSessionId() === id) {
      const remaining = sessions.filter((s) => s.id !== id);
      if (remaining.length > 0) {
        setActiveSessionId(remaining[remaining.length - 1].id);
      }
    }
    void appSessionApi.remove(id).catch(() => {});
  };

  onMount(() => {
    const handlers: Array<() => void> = [];
    let startupRaf: number | null = null;
    startupRaf = window.requestAnimationFrame(() => {
      startupRaf = null;
      void bootstrapApp();
    });

    const pushMessageToSession = (sid: string, role: string, text: string) => {
      appendMessageToSession(sid, { id: `${now()}-${Math.random().toString(36).slice(2)}`, roleName: role, text, at: now() });
    };

    void registerAcpEventListeners({
      acceptingStreams,
      sessions,
      appendStream,
      pushMessageToSession,
      pushMessage,
      onSessionDeltaLine: (sid, line) => {
        streamEngine.pendingSessionEvents.push({ sid, line });
        scheduleSessionEventFlush();
      },
      updateSession,
      mutateSession: (sessionId, recipe) =>
        mutateSessionWithProduce(sessionId, setSessions, recipe),
      appendThought,
      normalizeToolLocations,
      parseAgentCommands,
      normalizeRuntimeKey,
      roles,
      commandCacheKey,
      scheduleScrollToBottom,
    }).then((hs) => handlers.push(...hs));

    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setShowDrawer((v) => !v);
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "m") {
        e.preventDefault();
        setShowManagement((v) => !v);
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);

    onCleanup(() => {
      if (startupRaf !== null) {
        window.cancelAnimationFrame(startupRaf);
        startupRaf = null;
      }
      window.removeEventListener("keydown", handleGlobalKeyDown);
      dropStream();
      if (mentionCloseTimerRef.current !== null) window.clearTimeout(mentionCloseTimerRef.current);
      if (mentionDebounceTimerRef.current !== null) window.clearTimeout(mentionDebounceTimerRef.current);

      scheduleScrollToBottom(); // flushes scrollRaf reference via sessionManager
      closeMentionMenu();
      closeSlashMenu();
      handlers.forEach((h) => h());
    });
  });

  return (
    <div
      class="window-bg h-dvh overflow-hidden text-[var(--ui-text)] relative flex flex-col"
      onContextMenu={(e) => e.preventDefault()}
    >
      <div class="flex flex-1 flex-col min-h-0" style={{ "background-color": "var(--ui-bg)", "background-image": "radial-gradient(ellipse 80% 50% at 50% 0%, var(--ui-accent-soft), rgba(255,255,255,0))" }}>
      <SessionTabs
        sessions={sessions}
        activeSessionId={activeSessionId}
        setActiveSessionId={setActiveSessionId}
        activeSession={activeSession}
        patchActiveSession={patchActiveSession}
        activeBackendRole={activeBackendRole}
        onNewSession={newSession}
        onCloseSession={closeSession}
        updateSession={updateSession}
        onRefresh={() => { void refreshAssistants(); void refreshRoles(); void refreshSkills(); }}
        onToggleDrawer={() => setShowDrawer((v) => !v)}
        onToggleManagement={() => setShowManagement((v) => !v)}
      />

      <MessageWindow
        activeSessionId={activeSessionId}
        activeSession={activeSession}
        patchActiveSession={patchActiveSession}
        onResetAgentContext={resetActiveAgentContext}
        onReconnectAgent={reconnectActiveAgent}
        onListMounted={onListMounted}
        onListUnmounted={onListUnmounted}
      />
      </div>

      <ChatInput
        input={input}
        setInput={setInput}
        activeSession={activeSession}
        patchActiveSession={patchActiveSession}
        isCustomRole={isCustomRole}
        onSubmit={handleSend}
        onInputEvent={handleInputEvent}
        onInputKeyDown={handleInputKeyDownFinal}
        refreshInputCompletions={refreshInputCompletions}
        mentionOpen={mentionOpen}
        mentionItems={mentionItems}
        mentionActiveIndex={mentionActiveIndex}
        slashOpen={slashOpen}
        slashItems={slashItems}
        slashActiveIndex={slashActiveIndex}
        applyMentionCandidate={applyMentionCandidate}
        applySlashCandidate={applySlashCandidate}
        closeMentionMenu={closeMentionMenu}
        closeSlashMenu={closeSlashMenu}
        inputElRef={(el) => { inputEl = el; }}
        mentionCloseTimerRef={mentionCloseTimerRef}
        mentionDebounceTimerRef={mentionDebounceTimerRef}
      />

      <Show when={showDrawer()}>
        <Suspense fallback={null}>
          <ConfigDrawer
            showDrawer={showDrawer}
            setShowDrawer={setShowDrawer}
            assistants={assistants}
            roles={roles}
            skills={skills}
            activeSession={activeSession}
            patchActiveSession={patchActiveSession}
            sendRaw={sendRaw}
            refreshRoles={refreshRoles}
            refreshSkills={refreshSkills}
            refreshAssistants={refreshAssistants}
            pushMessage={pushMessage}
            fetchConfigOptions={fetchConfigOptions}
            fetchModes={fetchModes}
            onOpenManagement={(tab, roleName) => {
              setManagementInitialTab(tab ?? "sessions");
              setManagementInitialRole(roleName);
              setShowManagement(true);
            }}
            uiTheme={uiTheme}
            setUiTheme={(th) => {
              setUiTheme(th);
              window.localStorage.setItem(UI_THEME_KEY, th);
              document.documentElement.setAttribute("data-theme", th);
            }}
          />
        </Suspense>
      </Show>

      <Show when={showManagement()}>
        <Suspense fallback={null}>
          <ManagementPanel
            show={showManagement}
            onClose={() => setShowManagement(false)}
            initialTab={managementInitialTab()}
            initialRoleName={managementInitialRole()}
            activeSessions={sessions}
            onRestoreSession={(id, title, activeRole, runtimeKind, cwd) => {
              const existing = sessions.find((s) => s.id === id);
              if (!existing) {
                const s = makeDefaultSession(title);
                s.id = id;
                s.activeRole = activeRole;
                s.runtimeKind = runtimeKind;
                s.cwd = cwd;
                setSessions(sessions.length, s);
              }
              setActiveSessionId(id);
            }}
            skills={skills}
            roles={roles}
            refreshSkills={refreshSkills}
            activeSession={activeSession}
            patchActiveSession={patchActiveSession}
            refreshRoles={refreshRoles}
            fetchConfigOptions={fetchConfigOptions}
            pushMessage={pushMessage}
          />
        </Suspense>
      </Show>

      <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 pointer-events-none">
        <For each={toasts()}>
          {(t) => (
            <div class={`pointer-events-auto max-w-xs rounded-lg px-4 py-2 text-xs shadow-lg ${t.severity === "info" ? "theme-surface theme-text" : "bg-rose-900/90 text-rose-200"}`}>
              {t.message}
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
