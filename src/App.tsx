import { For, Show, Suspense, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";

import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import { now, DEFAULT_ROLE_ALIAS } from "./components/types";
import { UI_THEME_KEY } from "./lib/theme";
import SessionSidebar from "./components/SessionSidebar";
import WorkspaceHeader, { type WorkspaceToolPanel } from "./components/WorkspaceHeader";
import TerminalPanel from "./components/TerminalPanel";
import type { SettingsTab } from "./components/SettingsPage";
import WindowChrome from "./components/chrome/WindowChrome";
import AppShell from "./components/shell/AppShell";
import ToolPanelDock from "./components/shell/ToolPanelDock";

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
import type { RichNode } from "./components/RichInput";
import { getPlainText, isImageNode, placeCaretAfterChip } from "./components/RichInput";
import { useToast } from "./lib/useToast";
import { useTheme } from "./lib/useTheme";
import { useGitPoller } from "./hooks/useGitPoller";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";


const GitPanel = lazy(() => import("./components/GitPanel"));
const FilesPanel = lazy(() => import("./components/FilesPanel"));
const PreviewArea = lazy(() => import("./components/PreviewArea"));
const SettingsPage = lazy(() => import("./components/SettingsPage"));
import { useResize } from "./lib/useResize";
import { openPreviewTab, closePreviewTab, setActivePreviewTab, closeAllPreviewTabs, closeOtherPreviewTabs } from "./lib/previewTabs";

const TOOL_PANEL_KEY = "jockey:tool.panel";
const LEFT_RAIL_WIDTH_KEY = "jockey:leftRailWidth";
const RIGHT_TOOL_WIDTH_KEY = "jockey:rightToolPanelWidth";
const EDITOR_RATIO_KEY = "jockey:editorChatRatio";
const EDITOR_DEFAULT_RATIO = 0.6;
const EDITOR_MIN_RATIO = 0.15;
const EDITOR_MAX_RATIO = 0.85;
const LEFT_RAIL_DEFAULT_WIDTH = 326;
const LEFT_RAIL_MIN_WIDTH = 260;
const LEFT_RAIL_MAX_WIDTH = 420;
const RIGHT_TOOL_DEFAULT_WIDTH = 432;
const RIGHT_TOOL_MIN_WIDTH = 320;
const RIGHT_TOOL_MAX_WIDTH = 620;

export default function App() {
  const { toasts, showToast } = useToast();
  const { uiTheme, setUiTheme } = useTheme();

  const [showSettings, setShowSettings] = createSignal(false);
  const [settingsInitialTab, setSettingsInitialTab] = createSignal<SettingsTab>("general");
  const [settingsInitialRole, setSettingsInitialRole] = createSignal<string | undefined>(undefined);
  const [leftRailOpen, setLeftRailOpen] = createSignal(true);

  const initialPanelWidth = (key: string, fallback: number, min: number, max: number): number => {
    try {
      const raw = window.localStorage.getItem(key);
      const n = raw ? parseFloat(raw) : NaN;
      if (Number.isFinite(n)) return Math.min(max, Math.max(min, n));
    } catch { /* ignore */ }
    return fallback;
  };

  const initialToolPanel = (): WorkspaceToolPanel | null => null;
  const initialEditorRatio = (): number => {
    try {
      const raw = window.localStorage.getItem(EDITOR_RATIO_KEY);
      const n = raw ? parseFloat(raw) : NaN;
      if (Number.isFinite(n)) return Math.min(EDITOR_MAX_RATIO, Math.max(EDITOR_MIN_RATIO, n));
    } catch { /* ignore */ }
    return EDITOR_DEFAULT_RATIO;
  };

  const [toolPanel, setToolPanelInternal] = createSignal<WorkspaceToolPanel | null>(initialToolPanel());
  const [terminalCommandRequest, setTerminalCommandRequest] = createSignal<{ id: number; command: string } | null>(null);
  const [leftRailWidth, setLeftRailWidth] = createSignal(initialPanelWidth(LEFT_RAIL_WIDTH_KEY, LEFT_RAIL_DEFAULT_WIDTH, LEFT_RAIL_MIN_WIDTH, LEFT_RAIL_MAX_WIDTH));
  const [rightToolWidth, setRightToolWidth] = createSignal(initialPanelWidth(RIGHT_TOOL_WIDTH_KEY, RIGHT_TOOL_DEFAULT_WIDTH, RIGHT_TOOL_MIN_WIDTH, RIGHT_TOOL_MAX_WIDTH));
  const [editorRatio, setEditorRatio] = createSignal(initialEditorRatio());
  const [splitContainerEl, setSplitContainerEl] = createSignal<HTMLDivElement | null>(null);
  const [splitContainerHeight, setSplitContainerHeight] = createSignal(0);

  const setToolPanel = (p: WorkspaceToolPanel | null) => {
    setToolPanelInternal(p);
    try {
      if (p !== null) window.localStorage.setItem(TOOL_PANEL_KEY, p);
    } catch { /* ignore */ }
  };
  const persistEditorRatio = (r: number) => {
    const clamped = Math.min(EDITOR_MAX_RATIO, Math.max(EDITOR_MIN_RATIO, r));
    setEditorRatio(clamped);
    try { window.localStorage.setItem(EDITOR_RATIO_KEY, String(clamped)); } catch { /* ignore */ }
  };
  const persistLeftRailWidth = (px: number) => {
    const clamped = Math.min(LEFT_RAIL_MAX_WIDTH, Math.max(LEFT_RAIL_MIN_WIDTH, px));
    setLeftRailWidth(clamped);
    try { window.localStorage.setItem(LEFT_RAIL_WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };
  const persistRightToolWidth = (px: number) => {
    const clamped = Math.min(RIGHT_TOOL_MAX_WIDTH, Math.max(RIGHT_TOOL_MIN_WIDTH, px));
    setRightToolWidth(clamped);
    try { window.localStorage.setItem(RIGHT_TOOL_WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };

  const insertMentionAtCaret = (p: string) => {
    const mention = `@${p} `;
    setRichNodes((prev) => {
      const nodes = prev.length > 0 ? prev : [{ kind: "text" as const, text: "" }];
      const last = nodes[nodes.length - 1];
      if (last.kind === "text") {
        return [...nodes.slice(0, -1), { kind: "text" as const, text: last.text + mention }];
      }
      return [...nodes, { kind: "text" as const, text: mention }];
    });
    queueMicrotask(() => richInputEl?.focus());
  };

  const editorResize = useResize({
    axis: "y",
    min: 80,
    max: 4000,
    getStart: () => Math.round(editorRatio() * splitContainerHeight()),
    onCommit: (px) => {
      const h = splitContainerHeight();
      if (h > 0) persistEditorRatio(px / h);
    },
  });
  const leftRailResize = useResize({
    axis: "x",
    min: LEFT_RAIL_MIN_WIDTH,
    max: LEFT_RAIL_MAX_WIDTH,
    getStart: () => leftRailWidth(),
    onCommit: persistLeftRailWidth,
  });
  const rightToolResize = useResize({
    axis: "x",
    min: RIGHT_TOOL_MIN_WIDTH,
    max: RIGHT_TOOL_MAX_WIDTH,
    getStart: () => rightToolWidth(),
    onCommit: persistRightToolWidth,
    invert: true,
  });

  const openSettings = (tab: SettingsTab = "general", roleName?: string) => {
    setSettingsInitialTab(tab);
    setSettingsInitialRole(roleName);
    setShowSettings(true);
  };

  const [richNodes, setRichNodes] = createSignal<RichNode[]>([]);
  let richInputEl: HTMLDivElement | undefined;

  const input = () => getPlainText(richNodes());
  const setInput = (v: string) => setRichNodes([{ kind: "text", text: v }]);
  const setChatInput = (value: string | ((prev: string) => string)) => {
    const next = typeof value === "function" ? value(input()) : value;
    setInput(next);
    return next;
  };

  const fakeInputEl = (): HTMLInputElement => {
    const plain = input();
    return {
      value: plain,
      selectionStart: plain.length,
      focus() { richInputEl?.focus(); },
      setSelectionRange() {},
    } as unknown as HTMLInputElement;
  };

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

  const { gitChangeCount, gitStatus, refetch: refetchGitStatus } = useGitPoller(activeSession);

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
    fetchRoleConfig, fetchConfigOptions,
    parseAgentCommands,
    refreshAssistants,
    resetActiveAgentContext,
    reconnectActiveAgent,
  } = agentContext;

  const completions = useCompletions(
    agentContext,
    sessionManager,
    input,
    (v) => {
      setRichNodes((prev) => {
        const imgs = prev.filter(isImageNode);
        return imgs.length ? [...imgs, { kind: "text", text: v }] : [{ kind: "text", text: v }];
      });
    },
    fakeInputEl,
  );
  const {
    mentionOpen, mentionItems, mentionActiveIndex,
    slashOpen, slashItems, slashActiveIndex,
    mentionCloseTimerRef, mentionDebounceTimerRef,
    closeMentionMenu, closeSlashMenu,
    refreshInputCompletions,
    applyMentionCandidate, applySlashCandidate,
  } = completions;
  const { registerAcpEventListeners, clearSessionStream } = useAcpEventListeners();

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

  const { sendRaw, cancelCurrentRun } = useMessageSend({
    sessionManager,
    streamEngine,
    agentContext,
    closeMentionMenu,
    closeSlashMenu,
    showToast,
    clearSessionStream,
  });

  const inputHistory = useInputHistory(setInput);

  const chatActiveRole = createMemo(() => activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS);
  const chatSubmitting = createMemo(() => activeSession()?.submitting ?? false);
  const chatQueuedCount = createMemo(() => activeSession()?.queuedMessages.length ?? 0);

  const handlePasteImage = (items: DataTransferItemList, currentNodes: RichNode[]) => {
    const imageItems = Array.from(items).filter((it) => it.kind === "file" && it.type.startsWith("image/"));
    imageItems.forEach((item) => {
      const file = item.getAsFile();
      if (!file) return;
      const mimeType = file.type;
      const reader = new FileReader();
      reader.onload = (ev) => {
        const result = ev.target?.result as string | undefined;
        if (!result) return;
        const base64 = result.split(",")[1];
        if (!base64) return;
        let chipIndex = -1;
        setRichNodes((prev) => {
          const nodes = prev.length > 0 ? prev : currentNodes;
          chipIndex = nodes.filter(isImageNode).length;
          const newChip: RichNode = { kind: "image", index: chipIndex, img: { data: base64, mimeType } };
          return [...nodes, newChip, { kind: "text", text: " " }];
        });
        queueMicrotask(() => {
          if (richInputEl) placeCaretAfterChip(richInputEl, chipIndex);
        });
      };
      reader.readAsDataURL(file);
    });
  };

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const nodes = richNodes();
    const text = getPlainText(nodes).trim();
    const imageNodes = nodes.filter(isImageNode);
    if (!text && imageNodes.length === 0) return;
    if (activeSession()?.submitting) {
      if (imageNodes.length > 0) {
        showToast("Images can't be queued and will be dropped.", "info");
      }
      setRichNodes([]);
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
    setRichNodes([]);
    const attachments = imageNodes.map((n) => n.img);
    await sendRaw(text, false, null, attachments.length > 0 ? attachments : undefined, attachments.length > 0 ? attachments : undefined);
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

  const toggleToolPanel = (panel: WorkspaceToolPanel) =>
    setToolPanel(toolPanel() === panel ? null : panel);
  const runToolbarAction = (command: string) => {
    setToolPanel("terminal");
    setTerminalCommandRequest({ id: Date.now(), command });
  };

  const toggleToolPanelRestore = () => {
    if (toolPanel() !== null) { setToolPanel(null); return; }
    let remembered: WorkspaceToolPanel | null = null;
    try {
      const raw = window.localStorage.getItem(TOOL_PANEL_KEY);
      if (raw === "git" || raw === "files" || raw === "terminal" || raw === "commit") remembered = raw;
    } catch { }
    setToolPanel(remembered ?? "files");
  };

  useKeyboardShortcuts({
    newSession: () => showSettings() ? setShowSettings(false) : openSettings("general"),
    toggleManagement: () => openSettings("archived"),
    toggleSidebarRestore: toggleToolPanelRestore,
    setSidebarPanel: (p) => { if (p !== null) toggleToolPanel(p); },
    cancelCurrentRun: () => { void cancelCurrentRun(); },
  });

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

    onCleanup(() => {
      if (startupRaf !== null) {
        window.cancelAnimationFrame(startupRaf);
        startupRaf = null;
      }
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
    <AppShell
      showSettings={showSettings()}
      settings={
        <Suspense fallback={<div class="h-dvh theme-bg" />}>
          <SettingsPage
            initialTab={settingsInitialTab()}
            initialRoleName={settingsInitialRole()}
            uiTheme={uiTheme}
            setUiTheme={(th) => {
              setUiTheme(th);
              window.localStorage.setItem(UI_THEME_KEY, th);
              document.documentElement.setAttribute("data-theme", th);
            }}
            assistants={assistants}
            roles={roles}
            skills={skills}
            activeSessions={sessions}
            activeSession={activeSession}
            patchActiveSession={patchActiveSession}
            updateSession={updateSession}
            refreshSkills={refreshSkills}
            refreshRoles={refreshRoles}
            fetchRoleConfig={fetchRoleConfig}
            pushMessage={pushMessage}
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
              setShowSettings(false);
            }}
            onBack={() => setShowSettings(false)}
          />
        </Suspense>
      }
      chrome={
        <WindowChrome
          leftRailOpen={leftRailOpen()}
          onToggleLeftRail={() => setLeftRailOpen((v) => !v)}
        />
      }
      sidebar={
        <Show when={leftRailOpen()}>
          <SessionSidebar
            sessions={sessions}
            activeSessionId={activeSessionId}
            widthPx={leftRailWidth}
            setActiveSessionId={setActiveSessionId}
            onNewSession={newSession}
            onCloseSession={closeSession}
            updateSession={updateSession}
            onOpenAutomations={() => openSettings("automations")}
            onOpenSettings={() => openSettings("general")}
          />
          <div
            class="resizer-x sidebar-resizer"
            onMouseDown={leftRailResize.beginResize}
            title="Drag to resize sidebar"
          />
          <Show when={leftRailResize.previewPx() !== null}>
            <div
              class="pointer-events-none fixed bottom-0 top-0 w-px bg-[var(--ui-accent)] opacity-70 z-[70]"
              style={{ left: `${leftRailResize.previewPx() ?? 0}px` }}
            />
          </Show>
        </Show>
      }
      header={
        <WorkspaceHeader
          sessions={sessions}
          activeSessionId={activeSessionId}
          setActiveSessionId={setActiveSessionId}
          activeSession={activeSession}
          leftRailOpen={leftRailOpen}
          roles={roles}
          assistants={assistants}
          gitStatus={gitStatus}
          gitChangeCount={gitChangeCount}
          activeToolPanel={toolPanel}
          onNewSession={newSession}
          onCloseSession={closeSession}
          onToggleToolPanel={toggleToolPanel}
          onCancelRun={() => { void cancelCurrentRun(); }}
          onRunAction={runToolbarAction}
          onSelectRole={(roleName) => {
            if (roleName === DEFAULT_ROLE_ALIAS) {
              patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS, discoveredConfigOptions: [] });
              return;
            }
            const role = roles().find((r) => r.roleName === roleName);
            patchActiveSession({
              activeRole: roleName,
              runtimeKind: role?.runtimeKind ?? activeSession()?.runtimeKind ?? null,
              discoveredConfigOptions: [],
            });
            if (role) {
              void fetchConfigOptions(role.runtimeKind, role.roleName).then((opts) => {
                patchActiveSession({ discoveredConfigOptions: opts });
              });
            }
          }}
        />
      }
      preview={<></>}
      messages={
        <div class="main-work-area flex flex-1 flex-col min-h-0">
          <div
            class="flex flex-1 flex-col min-h-0 relative"
            ref={(el) => {
              setSplitContainerEl(el);
              const ro = new ResizeObserver((entries) => {
                for (const e of entries) setSplitContainerHeight(e.contentRect.height);
              });
              ro.observe(el);
              onCleanup(() => ro.disconnect());
              setSplitContainerHeight(el.clientHeight);
            }}
          >
            <Show when={(activeSession()?.previewTabs.length ?? 0) > 0}>
              <div
                class="preview-shell shrink-0 overflow-hidden"
                style={{ height: `${Math.round(editorRatio() * splitContainerHeight())}px` }}
              >
                <Suspense fallback={<div class="flex-1 theme-bg" />}>
                  <PreviewArea
                    session={activeSession}
                    appSessionId={() => activeSession()?.id}
                    onCloseTab={(tabId) => {
                      const sid = activeSessionId();
                      if (sid) closePreviewTab(mutateSession, sid, tabId);
                    }}
                    onCloseOthers={(tabId) => {
                      const sid = activeSessionId();
                      if (sid) closeOtherPreviewTabs(mutateSession, sid, tabId);
                    }}
                    onCloseAll={() => {
                      const sid = activeSessionId();
                      if (sid) closeAllPreviewTabs(mutateSession, sid);
                    }}
                    onActivateTab={(tabId) => {
                      const sid = activeSessionId();
                      if (sid) setActivePreviewTab(mutateSession, sid, tabId);
                    }}
                    onAddMention={insertMentionAtCaret}
                  />
                </Suspense>
              </div>
              <div
                class="resizer-y"
                onMouseDown={editorResize.beginResize}
                title="Drag to resize"
              />
              <Show when={editorResize.previewPx() !== null}>
                <div
                  class="pointer-events-none fixed left-0 right-0 h-px bg-[var(--ui-accent)] opacity-70 z-[70]"
                  style={{
                    top: `${(splitContainerEl()?.getBoundingClientRect().top ?? 0) + (editorResize.previewPx() ?? 0)}px`,
                  }}
                />
              </Show>
            </Show>

            <div class="flex flex-1 flex-col min-h-0">
              <MessageWindow
                activeSessionId={activeSessionId}
                activeSession={activeSession}
                activeBackendRole={activeBackendRole}
                patchActiveSession={patchActiveSession}
                onRemoveQueuedMessage={(index) => {
                  const sid = activeSessionId();
                  if (!sid) return;
                  const idx = getSessionIndex(sid);
                  if (idx === -1) return;
                  setSessions(idx, "queuedMessages", (prev) => prev.filter((_, i) => i !== index));
                }}
                onFlushQueue={() => { void cancelCurrentRun(); }}
                onResetAgentContext={resetActiveAgentContext}
                onReconnectAgent={reconnectActiveAgent}
                onListMounted={onListMounted}
                onListUnmounted={onListUnmounted}
                onFileClick={(path, kind) => {
                  const sid = activeSessionId();
                  const cwd = activeSession()?.cwd ?? "";
                  if (!sid || !cwd) return;
                  const isEdit = kind === "write" || kind === "edit" || kind === "create" || kind === "patch";
                  openPreviewTab(mutateSession, sid, {
                    cwd, path, initialMode: isEdit ? "diff" : "file", staged: false, untracked: false,
                  });
                }}
                onRejectHunk={(rejectPrompt) => {
                  void sendRaw(rejectPrompt, false);
                }}
              />
            </div>
          </div>
        </div>
      }
      composer={
        <ChatInput
          input={input}
          setInput={setChatInput}
          richNodes={richNodes}
          setRichNodes={setRichNodes}
          activeRole={chatActiveRole}
          submitting={chatSubmitting}
          queuedCount={chatQueuedCount}
          onResetRole={() => patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS })}
          isCustomRole={isCustomRole}
          onSubmit={handleSend}
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
          richInputRef={(el) => { richInputEl = el; }}
          mentionCloseTimerRef={mentionCloseTimerRef}
          mentionDebounceTimerRef={mentionDebounceTimerRef}
          hasImages={() => richNodes().some(isImageNode)}
          onPasteImage={handlePasteImage}
          onRemoveImage={(removeIdx) => {
            setRichNodes((prev) => {
              const without = prev.filter((n) => !(isImageNode(n) && n.index === removeIdx));
              let imgCounter = 0;
              return without.map((n) => isImageNode(n) ? { ...n, index: imgCounter++ } : n);
            });
          }}
        />
      }
      rightDock={
        <ToolPanelDock
          open={toolPanel() !== null}
          widthPx={rightToolWidth()}
          previewPx={rightToolResize.previewPx()}
          onResizeStart={rightToolResize.beginResize}
        >
          <Suspense fallback={<div class="flex-1 bg-[var(--ui-sidebar-bg)]" />}>
            <Show when={toolPanel() === "git" || toolPanel() === "commit"}>
              <GitPanel
                appSessionId={() => activeSession()?.id}
                cwd={() => activeSession()?.cwd ?? null}
                gitStatus={gitStatus}
                onRefresh={refetchGitStatus}
                onAddMention={insertMentionAtCaret}
                onCollapse={() => setToolPanel(null)}
                onOpenDiff={(path, staged, untracked) => {
                  const sid = activeSessionId();
                  const cwd = activeSession()?.cwd ?? "";
                  if (!sid || !cwd) return;
                  openPreviewTab(mutateSession, sid, {
                    cwd, path, initialMode: "diff", staged, untracked,
                  });
                }}
              />
            </Show>
            <Show when={toolPanel() === "files"}>
              <FilesPanel
                appSessionId={() => activeSession()?.id}
                cwd={() => activeSession()?.cwd ?? null}
                onOpenFile={(path) => {
                  const sid = activeSessionId();
                  const cwd = activeSession()?.cwd ?? "";
                  if (!sid || !cwd) return;
                  openPreviewTab(mutateSession, sid, {
                    cwd, path, initialMode: "file", staged: false, untracked: false,
                  });
                }}
                onCollapse={() => setToolPanel(null)}
              />
            </Show>
            <Show when={toolPanel() === "terminal"}>
              <TerminalPanel
                session={activeSession()}
                commandRequest={terminalCommandRequest()}
                onClose={() => setToolPanel(null)}
              />
            </Show>
          </Suspense>
        </ToolPanelDock>
      }
      toasts={
        <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 pointer-events-none">
          <For each={toasts()}>
            {(t) => (
              <div class="jockey-toast pointer-events-auto" classList={{ "is-info": t.severity === "info", "is-danger": t.severity !== "info" }}>
                {t.message}
              </div>
            )}
          </For>
        </div>
      }
    />
  );
}
