import { For, Show, Suspense, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";

import SessionTabs, { type SessionTab } from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import { now } from "./components/types";
import { type UiTheme, normalizeUiTheme, UI_THEME_KEY } from "./lib/theme";

import { useSessionManager } from "./hooks/useSessionManager";
import { useStreamEngine } from "./hooks/useStreamEngine";
import { useAgentContext } from "./hooks/useAgentContext";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useAcpEventListeners } from "./hooks/useAcpEventListeners";
import { useCompletions } from "./hooks/useCompletions";
import { useMessageSend } from "./hooks/useMessageSend";
import { useInputHistory } from "./hooks/useInputHistory";
import { uniqueName, makeDefaultSession } from "./lib/sessionHelpers";
import { createSessionEventBuffer } from "./lib/sessionEventBuffer";
import { appSessionApi } from "./lib/tauriApi";


const ConfigDrawer = lazy(() => import("./components/ConfigDrawer"));
const ManagementPanel = lazy(() => import("./components/ManagementPanel"));
const GitPanel = lazy(() => import("./components/GitPanel"));
const GitDiffModal = lazy(() => import("./components/GitDiffModal"));

const GIT_PANEL_WIDTH_KEY = "jockey:git-panel-width";
const GIT_PANEL_OPEN_KEY = "jockey:git-panel-open";
const GIT_PANEL_DEFAULT_WIDTH = 280;
const GIT_PANEL_MIN_WIDTH = 200;
const GIT_PANEL_MAX_WIDTH = 520;

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

  const initialGitPanelOpen = (): boolean => {
    try { return window.localStorage.getItem(GIT_PANEL_OPEN_KEY) === "1"; } catch { return false; }
  };
  const initialGitPanelWidth = (): number => {
    try {
      const raw = window.localStorage.getItem(GIT_PANEL_WIDTH_KEY);
      const n = raw ? parseInt(raw, 10) : NaN;
      if (Number.isFinite(n)) return Math.min(GIT_PANEL_MAX_WIDTH, Math.max(GIT_PANEL_MIN_WIDTH, n));
    } catch { /* ignore */ }
    return GIT_PANEL_DEFAULT_WIDTH;
  };
  const [showGitPanel, setShowGitPanel] = createSignal(initialGitPanelOpen());
  const [gitPanelWidth, setGitPanelWidth] = createSignal(initialGitPanelWidth());

  const persistGitPanelOpen = (v: boolean) => {
    setShowGitPanel(v);
    try { window.localStorage.setItem(GIT_PANEL_OPEN_KEY, v ? "1" : "0"); } catch { /* ignore */ }
  };
  const persistGitPanelWidth = (w: number) => {
    const clamped = Math.min(GIT_PANEL_MAX_WIDTH, Math.max(GIT_PANEL_MIN_WIDTH, w));
    setGitPanelWidth(clamped);
    try { window.localStorage.setItem(GIT_PANEL_WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };

  type GitDiffTarget = { path: string; staged: boolean };
  const [gitDiffTarget, setGitDiffTarget] = createSignal<GitDiffTarget | null>(null);

  const insertMentionAtCaret = (p: string) => {
    const target = inputEl;
    const mention = `@${p}`;
    const current = input();
    if (target) {
      const caret = target.selectionStart ?? current.length;
      const left = current.slice(0, caret);
      const right = current.slice(caret);
      const needsLead = left.length > 0 && !left.endsWith(" ");
      const insert = `${needsLead ? " " : ""}${mention} `;
      const next = `${left}${insert}${right}`;
      setInput(next);
      const pos = caret + insert.length;
      queueMicrotask(() => {
        target.focus();
        target.setSelectionRange(pos, pos);
      });
    } else {
      setInput(current ? `${current} ${mention} ` : `${mention} `);
    }
  };

  const beginGitPanelResize = (startEvent: MouseEvent) => {
    startEvent.preventDefault();
    const startX = startEvent.clientX;
    const startWidth = gitPanelWidth();
    const onMove = (ev: MouseEvent) => {
      const delta = ev.clientX - startX;
      const next = startWidth + delta;
      setGitPanelWidth(Math.min(GIT_PANEL_MAX_WIDTH, Math.max(GIT_PANEL_MIN_WIDTH, next)));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      try { window.localStorage.setItem(GIT_PANEL_WIDTH_KEY, String(gitPanelWidth())); } catch { /* ignore */ }
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };
  const [managementInitialTab, setManagementInitialTab] = createSignal<"sessions" | "workflows" | "roles" | "mcp" | "skills">("sessions");
  const [managementInitialRole, setManagementInitialRole] = createSignal<string | undefined>(undefined);

  const [input, setInput] = createSignal("");
  let inputEl: HTMLInputElement | undefined;

  const sessionManager = useSessionManager();
  const {
    sessions, setSessions,
    activeSessionId, setActiveSessionId,
    activeSession,
    updateSession, mutateSession, patchActiveSession,
    appendMessageToSession, pushMessage,
    scheduleScrollToBottom,
    onListMounted, onListUnmounted,
    getSessionIndex,
  } = sessionManager;

  // Minimal projection for SessionTabs — only re-computes when id/title/status change,
  // not on every streaming delta. Prevents full For reconciliation on each stream update.
  const sessionTabs = createMemo<SessionTab[]>(() =>
    sessions.map((s) => ({ id: s.id, title: s.title, status: s.status }))
  );

  const streamEngine = useStreamEngine(sessionManager);
  const {
    acceptingStreams, streamBatchBuffers, thoughtBatchBuffers,
    appendStream, appendThought,
    dropStream,
    normalizeToolLocations,
  } = streamEngine;

  const sessionEventBuffer = createSessionEventBuffer({
    sessions,
    setSessions,
    getSessionIndex: sessionManager.getSessionIndex,
    pushMessage: sessionManager.pushMessage,
  });

  const agentContext = useAgentContext(sessionManager, streamEngine, showToast);
  const {
    roles, assistants, skills,
    normalizeRuntimeKey, commandCacheKey,
    isCustomRole, activeBackendRole,
    refreshRoles, refreshSkills,
    fetchConfigOptions, fetchModes,
    parseAgentCommands,
    refreshAssistants,
    resetActiveAgentContext,
    reconnectActiveAgent,
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

  const { sendRaw, runNextQueued, cancelCurrentRun } = useMessageSend({
    sessionManager,
    streamEngine,
    agentContext,
    closeMentionMenu,
    closeSlashMenu,
    showToast,
  });

  const inputHistory = useInputHistory(setInput);

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const text = input().trim();
    if (!text) return;
    if (activeSession()?.submitting) {
      setInput("");
      const sid = activeSessionId();
      if (sid) {
        const qidx = getSessionIndex(sid);
        if (qidx !== -1) setSessions(qidx, "queuedMessages", (prev) => [...prev, text]);
        scheduleScrollToBottom();
      }
      return;
    }
    if (!activeSession()?.runtimeKind && !isCustomRole() && !text.startsWith("/app_")) {
      pushMessage("system", "Select an assistant or a role first.");
      return;
    }
    inputHistory.push(text);
    setInput("");
    await sendRaw(text);
  };

  const handleInputEvent = (el: HTMLInputElement) => {
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    setInput(value);
    inputHistory.resetIndex();
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

    if (inputHistory.handleKey(e, input)) {
      e.preventDefault();
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
    if (activeSessionId() === id) {
      setActiveSessionId(remaining[remaining.length - 1].id);
    }
    setSessions(remaining);
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
      getSessionIndex,
      appendStream,
      pushMessageToSession,
      pushMessage,
      onSessionDeltaLine: (sid, line) => sessionEventBuffer.push(sid, line),
      updateSession,
      mutateSession,
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
      if ((e.metaKey || e.ctrlKey) && e.key === "g") {
        e.preventDefault();
        persistGitPanelOpen(!showGitPanel());
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
      <div class="flex flex-1 flex-row min-h-0 min-w-0">
        <Show when={showGitPanel()}>
          <div
            class="shrink-0 border-r theme-border flex flex-col min-h-0"
            style={{ width: `${gitPanelWidth()}px` }}
          >
            <Suspense fallback={<div class="flex-1 theme-surface" />}>
              <GitPanel
                appSessionId={() => activeSession()?.id}
                onAddMention={insertMentionAtCaret}
                onCollapse={() => persistGitPanelOpen(false)}
                onOpenDiff={(path, staged) => setGitDiffTarget({ path, staged })}
              />
            </Suspense>
          </div>
          <div
            class="w-1 shrink-0 cursor-col-resize hover:bg-[var(--ui-accent-soft)] active:bg-[var(--ui-accent-soft)]"
            onMouseDown={beginGitPanelResize}
            title="Drag to resize"
          />
        </Show>

        <div class="flex flex-1 flex-col min-h-0 min-w-0">
          <div class="flex flex-1 flex-col min-h-0" style={{ "background-color": "var(--ui-bg)", "background-image": "radial-gradient(ellipse 80% 50% at 50% 0%, var(--ui-accent-soft), rgba(255,255,255,0))" }}>
            <SessionTabs
              sessions={sessionTabs()}
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
              onToggleGitPanel={() => persistGitPanelOpen(!showGitPanel())}
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
        </div>
      </div>

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
              const existing = getSessionIndex(id) !== -1;
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

      <Show when={gitDiffTarget()}>
        {(target) => (
          <Suspense fallback={null}>
            <GitDiffModal
              appSessionId={() => activeSession()?.id}
              path={target().path}
              staged={target().staged}
              onClose={() => setGitDiffTarget(null)}
              onAddMention={insertMentionAtCaret}
            />
          </Suspense>
        )}
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
