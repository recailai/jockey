import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";

import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import { now, DEFAULT_ROLE_ALIAS } from "./components/types";
import { UI_THEME_KEY } from "./lib/theme";

import SessionTopbar from "./components/chrome/SessionTopbar";
import ComposerContextFooter from "./components/ComposerContextFooter";
import ConversationCanvas from "./components/ConversationCanvas";
import type { SettingsTab } from "./components/SettingsPage";
import AppShell from "./components/shell/AppShell";
import RightToolDock from "./components/shell/RightToolDock";
import ToolDockPanels from "./components/shell/ToolDockPanels";
import {
  LAYOUT_STORAGE,
  PREVIEW,
  RIGHT_DOCK,
  initialPreviewRatio,
  initialRightDockOpen,
  initialRightDockWidth,
  initialRightPanel,
  type RightDockPanel,
} from "./lib/layoutTokens";
import { hasConversationContent } from "./lib/conversationHelpers";

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
import {
  getPlainText,
  getRichInputCaretOffset,
  insertTextIntoRichNodes,
  isImageNode,
  placeCaretAfterChip,
  setRichInputSelection,
} from "./components/RichInput";
import { useToast } from "./lib/useToast";
import { useTheme } from "./lib/useTheme";
import { useGitPoller } from "./hooks/useGitPoller";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";


const SettingsPage = lazy(() => import("./components/SettingsPage"));
import { useResize } from "./lib/useResize";
import { openPreviewTab, closePreviewTab, setActivePreviewTab, closeAllPreviewTabs, closeOtherPreviewTabs } from "./lib/previewTabs";
import { destroySessionTerminal, updateTerminalThemes } from "./lib/terminalRuntime";

export default function App() {
  const { toasts, showToast } = useToast();
  const { uiTheme, setUiTheme } = useTheme();

  createEffect(() => {
    uiTheme();
    setTimeout(() => {
      updateTerminalThemes();
    }, 50);
  });

  const [showSettings, setShowSettings] = createSignal(false);
  const [settingsInitialTab, setSettingsInitialTab] = createSignal<SettingsTab>("general");
  const [settingsInitialRole, setSettingsInitialRole] = createSignal<string | undefined>(undefined);

  const [rightDockOpen, setRightDockOpenInternal] = createSignal(initialRightDockOpen());
  const [rightDockPanel, setRightDockPanelInternal] = createSignal<RightDockPanel | null>(
    initialRightDockOpen() ? (initialRightPanel() ?? "git") : null,
  );
  const [terminalCommandRequest, setTerminalCommandRequest] = createSignal<{ id: number; command: string } | null>(null);
  const [rightDockWidth, setRightDockWidth] = createSignal(initialRightDockWidth());
  const [editorRatio, setEditorRatio] = createSignal(initialPreviewRatio());
  const [splitContainerEl, setSplitContainerEl] = createSignal<HTMLDivElement | null>(null);
  const [splitContainerHeight, setSplitContainerHeight] = createSignal(0);

  const setRightDockOpen = (open: boolean) => {
    setRightDockOpenInternal(open);
    try { window.localStorage.setItem(LAYOUT_STORAGE.rightDockOpen, open ? "1" : "0"); } catch { /* ignore */ }
  };
  const setRightDockPanel = (panel: RightDockPanel | null) => {
    setRightDockPanelInternal(panel);
    try {
      if (panel !== null) window.localStorage.setItem(LAYOUT_STORAGE.rightPanel, panel);
      else window.localStorage.removeItem(LAYOUT_STORAGE.rightPanel);
    } catch { /* ignore */ }
  };
  const persistEditorRatio = (r: number) => {
    const clamped = Math.min(PREVIEW.maxRatio, Math.max(PREVIEW.minRatio, r));
    setEditorRatio(clamped);
    try { window.localStorage.setItem(LAYOUT_STORAGE.previewRatio, String(clamped)); } catch { /* ignore */ }
  };
  const persistRightDockWidth = (px: number) => {
    const clamped = Math.min(RIGHT_DOCK.maxWidth, Math.max(RIGHT_DOCK.minWidth, px));
    setRightDockWidth(clamped);
    try { window.localStorage.setItem(LAYOUT_STORAGE.rightDockWidth, String(clamped)); } catch { /* ignore */ }
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
  const rightDockResize = useResize({
    axis: "x",
    min: RIGHT_DOCK.minWidth,
    max: RIGHT_DOCK.maxWidth,
    getStart: () => rightDockWidth(),
    onCommit: persistRightDockWidth,
    invert: true,
  });

  const openSettings = (tab: SettingsTab = "general", roleName?: string) => {
    setSettingsInitialTab(tab);
    setSettingsInitialRole(roleName);
    setShowSettings(true);
  };

  const [richNodes, setRichNodes] = createSignal<RichNode[]>([]);
  const [richCaretOffset, setRichCaretOffset] = createSignal(0);
  let richInputEl: HTMLDivElement | undefined;

  const input = () => getPlainText(richNodes());
  const liveRichCaretOffset = () => {
    const live = richInputEl ? getRichInputCaretOffset(richInputEl) : -1;
    return live >= 0 ? live : richCaretOffset();
  };
  const restoreRichCaret = (start: number, end = start) => {
    setRichCaretOffset(end);
    if (!richInputEl) return;
    setRichInputSelection(richInputEl, start, end);
  };
  const restoreRichCaretSoon = (start: number, end = start) => {
    setRichCaretOffset(end);
    queueMicrotask(() => restoreRichCaret(start, end));
  };
  const setInput = (v: string) => {
    setRichNodes(v ? [{ kind: "text", text: v }] : []);
    restoreRichCaretSoon(v.length);
  };
  const fakeInputEl = (): HTMLInputElement => {
    const plain = input();
    const selection = () => liveRichCaretOffset();
    return {
      value: plain,
      get selectionStart() { return selection(); },
      get selectionEnd() { return selection(); },
      focus() { richInputEl?.focus(); },
      setSelectionRange(start: number, end?: number | null) {
        restoreRichCaret(start, typeof end === "number" ? end : start);
      },
    } as unknown as HTMLInputElement;
  };

  const insertMentionAtCaret = (p: string) => {
    const mention = `@${p} `;
    let nextCaret = 0;
    setRichNodes((prev) => {
      const inserted = insertTextIntoRichNodes(prev, liveRichCaretOffset(), mention);
      nextCaret = inserted.caret;
      return inserted.nodes;
    });
    setRichCaretOffset(nextCaret);
    queueMicrotask(() => {
      richInputEl?.focus();
      restoreRichCaret(nextCaret);
    });
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
          if (!richInputEl) return;
          placeCaretAfterChip(richInputEl, chipIndex);
          const caret = getRichInputCaretOffset(richInputEl);
          if (caret >= 0) setRichCaretOffset(caret);
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
      setRichCaretOffset(0);
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
    setRichCaretOffset(0);
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
    void destroySessionTerminal(id);
    const remaining = sessions.filter((s) => s.id !== id);
    if (remaining.length === 0) { void appSessionApi.remove(id).catch(() => {}); return; }
    if (activeSessionId() === id) {
      setActiveSessionId(remaining[remaining.length - 1].id);
    }
    setSessions(remaining);
    void appSessionApi.remove(id).catch(() => {});
  };

  const openWorkspacePanel = (panel: RightDockPanel) => {
    setRightDockOpen(true);
    setRightDockPanel(panel);
  };
  const openTerminalPanel = (command?: string) => {
    setRightDockOpen(true);
    setRightDockPanel("terminal");
    if (command !== undefined) {
      setTerminalCommandRequest({ id: Date.now(), command });
    }
  };
  const showRightDockLauncher = () => setRightDockPanel(null);
  const runToolbarAction = (command: string) => {
    openTerminalPanel(command);
  };
  const openSessionPreview = (
    path: string,
    mode: "file" | "diff" | "commit",
    opts?: { staged?: boolean; untracked?: boolean; commitOid?: string; label?: string },
  ) => {
    const sid = activeSessionId();
    const cwd = activeSession()?.cwd ?? "";
    if (!sid || !cwd) return;
    openPreviewTab(mutateSession, sid, {
      cwd,
      path,
      initialMode: mode,
      staged: opts?.staged ?? false,
      untracked: opts?.untracked ?? false,
      commitOid: opts?.commitOid,
      label: opts?.label,
    });
  };

  const toggleRightDock = () => {
    if (rightDockOpen()) {
      setRightDockOpen(false);
      return;
    }
    setRightDockOpen(true);
    if (!rightDockPanel()) {
      setRightDockPanel(initialRightPanel() ?? "git");
    }
  };

  useKeyboardShortcuts({
    newSession,
    openSettings: () => (showSettings() ? setShowSettings(false) : openSettings("general")),
    toggleManagement: () => openSettings("archived"),
    toggleRightDock,
    openWorkspacePanel,
  });

  const composerBlock = (layout: "empty" | "active") => (
    <ChatInput
      layout={layout}
      richNodes={richNodes}
      setRichNodes={setRichNodes}
      activeRole={chatActiveRole}
      submitting={chatSubmitting}
      queuedCount={chatQueuedCount}
      onResetRole={() => patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS })}
      isCustomRole={isCustomRole}
      onSubmit={handleSend}
      onInputKeyDown={handleInputKeyDownFinal}
      refreshInputCompletions={(value, caret) => {
        setRichCaretOffset(caret);
        refreshInputCompletions(value, caret);
      }}
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
      contextFooter={
        <ComposerContextFooter
          activeSession={activeSession}
          roles={roles}
          assistants={assistants}
          gitStatus={gitStatus}
          gitChangeCount={gitChangeCount}
          activeToolPanel={rightDockPanel}
          onOpenToolPanel={openWorkspacePanel}
          onCancelRun={() => { void cancelCurrentRun(); }}
          onRunAction={runToolbarAction}
          onRefreshGit={refetchGitStatus}
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
    />
  );

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
            showToast={showToast}
          />
        </Suspense>
      }
      rightDock={
        <RightToolDock
          open={rightDockOpen()}
          activePanel={rightDockPanel()}
          widthPx={rightDockWidth()}
          previewPx={rightDockResize.previewPx()}
          onResizeStart={rightDockResize.beginResize}
          onPanelChange={openWorkspacePanel}
          onShowLauncher={showRightDockLauncher}
        >
          <ToolDockPanels
            activePanel={rightDockPanel()}
            dockEmbedded
            activeSession={activeSession}
            gitStatus={gitStatus}
            onRefreshGit={refetchGitStatus}
            onClose={showRightDockLauncher}
            onAddMention={insertMentionAtCaret}
            onOpenFile={(path) => openSessionPreview(path, "file")}
            onOpenDiff={(path, staged, untracked) => openSessionPreview(path, "diff", { staged, untracked })}
            onOpenCommitDiff={(oid, label) => openSessionPreview(`__commit__/${oid}`, "commit", { label, commitOid: oid })}
            terminalCommandRequest={terminalCommandRequest()}
          />
        </RightToolDock>
      }
      sessionTopbar={
        <SessionTopbar
          sessions={sessions}
          activeSessionId={activeSessionId}
          setActiveSessionId={setActiveSessionId}
          updateSession={updateSession}
          onNewSession={newSession}
          onCloseSession={closeSession}
          onOpenSettings={openSettings}
          onToggleRightDock={toggleRightDock}
          rightDockOpen={rightDockOpen}
        />
      }
      conversation={
        <ConversationCanvas
          activeSession={activeSession}
          activeSessionId={activeSessionId}
          mutateSession={mutateSession}
          editorRatio={editorRatio}
          splitContainerHeight={splitContainerHeight}
          splitContainerEl={splitContainerEl}
          setSplitContainerEl={setSplitContainerEl}
          setSplitContainerHeight={setSplitContainerHeight}
          editorResize={editorResize}
          insertMentionAtCaret={insertMentionAtCaret}
          messages={
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
          }
          composer={composerBlock(hasConversationContent(activeSession()) ? "active" : "empty")}
        />
      }
      toasts={
        <div class="jockey-toast-stack fixed bottom-4 right-4 flex flex-col gap-2 pointer-events-none">
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
