import { For, Show, Suspense, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";

import SessionTabs, { type SessionTab } from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import { now, DEFAULT_ROLE_ALIAS } from "./components/types";
import { UI_THEME_KEY } from "./lib/theme";

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


const ConfigDrawer = lazy(() => import("./components/ConfigDrawer"));
const ManagementPanel = lazy(() => import("./components/ManagementPanel"));
const GitPanel = lazy(() => import("./components/GitPanel"));
const FilesPanel = lazy(() => import("./components/FilesPanel"));
const PreviewArea = lazy(() => import("./components/PreviewArea"));
const StatusBar = lazy(() => import("./components/StatusBar"));
import ActivityBar, { type ActivityPanel } from "./components/ActivityBar";
import { useResize } from "./lib/useResize";
import { openPreviewTab, closePreviewTab, setActivePreviewTab, closeAllPreviewTabs, closeOtherPreviewTabs } from "./lib/previewTabs";

const SIDEBAR_WIDTH_KEY = "jockey:sidebar.width";
const SIDEBAR_PANEL_KEY = "jockey:sidebar.panel";
const SIDEBAR_DEFAULT_WIDTH = 280;
const SIDEBAR_MIN_WIDTH = 200;
const SIDEBAR_MAX_WIDTH = 520;
const EDITOR_RATIO_KEY = "jockey:editorChatRatio";
const EDITOR_DEFAULT_RATIO = 0.6;
const EDITOR_MIN_RATIO = 0.15;
const EDITOR_MAX_RATIO = 0.85;

export default function App() {
  const { toasts, showToast } = useToast();
  const { uiTheme, setUiTheme } = useTheme();

  const [showDrawer, setShowDrawer] = createSignal(false);
  const [showManagement, setShowManagement] = createSignal(false);

  // Sidebar is always hidden on startup; SIDEBAR_PANEL_KEY is only consulted by
  // Cmd/Ctrl+B restore to reopen the user's last-chosen panel.
  const initialSidebarPanel = (): ActivityPanel | null => null;
  const initialSidebarWidth = (): number => {
    try {
      const raw = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
      const n = raw ? parseInt(raw, 10) : NaN;
      if (Number.isFinite(n)) return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, n));
    } catch { /* ignore */ }
    return SIDEBAR_DEFAULT_WIDTH;
  };
  const initialEditorRatio = (): number => {
    try {
      const raw = window.localStorage.getItem(EDITOR_RATIO_KEY);
      const n = raw ? parseFloat(raw) : NaN;
      if (Number.isFinite(n)) return Math.min(EDITOR_MAX_RATIO, Math.max(EDITOR_MIN_RATIO, n));
    } catch { /* ignore */ }
    return EDITOR_DEFAULT_RATIO;
  };

  const [sidebarPanel, setSidebarPanelInternal] = createSignal<ActivityPanel | null>(initialSidebarPanel());
  const [sidebarWidth, setSidebarWidth] = createSignal(initialSidebarWidth());
  const [editorRatio, setEditorRatio] = createSignal(initialEditorRatio());
  const [splitContainerEl, setSplitContainerEl] = createSignal<HTMLDivElement | null>(null);
  const [splitContainerHeight, setSplitContainerHeight] = createSignal(0);

  const setSidebarPanel = (p: ActivityPanel | null) => {
    setSidebarPanelInternal(p);
    try {
      // Remember last-opened panel for Cmd/Ctrl+B restore; only clear when a new value is set.
      if (p !== null) window.localStorage.setItem(SIDEBAR_PANEL_KEY, p);
    } catch { /* ignore */ }
  };
  const persistSidebarWidth = (w: number) => {
    const clamped = Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, w));
    setSidebarWidth(clamped);
    try { window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };
  const persistEditorRatio = (r: number) => {
    const clamped = Math.min(EDITOR_MAX_RATIO, Math.max(EDITOR_MIN_RATIO, r));
    setEditorRatio(clamped);
    try { window.localStorage.setItem(EDITOR_RATIO_KEY, String(clamped)); } catch { /* ignore */ }
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

  const sidebarResize = useResize({
    axis: "x",
    min: SIDEBAR_MIN_WIDTH,
    max: SIDEBAR_MAX_WIDTH,
    getStart: () => sidebarWidth(),
    onCommit: persistSidebarWidth,
  });

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
  const [managementInitialTab, setManagementInitialTab] = createSignal<"sessions" | "workflows" | "roles" | "mcp" | "skills" | "rules" | "agents">("sessions");
  const [managementInitialRole, setManagementInitialRole] = createSignal<string | undefined>(undefined);

  const [richNodes, setRichNodes] = createSignal<RichNode[]>([]);
  let richInputEl: HTMLDivElement | undefined;

  const input = () => getPlainText(richNodes());
  const setInput = (v: string) => setRichNodes([{ kind: "text", text: v }]);

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

  const toggleSidebar = (panel: ActivityPanel) =>
    setSidebarPanel(sidebarPanel() === panel ? null : panel);

  const toggleSidebarRestore = () => {
    if (sidebarPanel() !== null) { setSidebarPanel(null); return; }
    let remembered: ActivityPanel | null = null;
    try {
      const raw = window.localStorage.getItem(SIDEBAR_PANEL_KEY);
      if (raw === "git" || raw === "files") remembered = raw;
    } catch { }
    setSidebarPanel(remembered ?? "files");
  };

  useKeyboardShortcuts({
    newSession: () => setShowDrawer((v) => !v),
    toggleManagement: () => setShowManagement((v) => !v),
    toggleSidebarRestore,
    setSidebarPanel: (p) => { if (p !== null) toggleSidebar(p); },
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
    <div
      class="window-bg h-dvh overflow-hidden text-[var(--ui-text)] relative flex flex-col"
      onContextMenu={(e) => e.preventDefault()}
    >
      <div class="flex flex-1 flex-row min-h-0 min-w-0">
        <ActivityBar activePanel={sidebarPanel} onSelect={(p) => setSidebarPanel(p)} gitChangeCount={gitChangeCount} />
        <Show when={sidebarPanel() !== null}>
          <div
            class="theme-sidebar-shell shrink-0 flex flex-col min-h-0"
            style={{ width: `${sidebarWidth()}px` }}
          >
            <div data-tauri-drag-region class="h-[34px] shrink-0" />
            <Suspense fallback={<div class="flex-1 theme-sidebar" />}>
              <Show when={sidebarPanel() === "git"}>
                <GitPanel
                  appSessionId={() => activeSession()?.id}
                  cwd={() => activeSession()?.cwd ?? null}
                  gitStatus={gitStatus}
                  onRefresh={refetchGitStatus}
                  onAddMention={insertMentionAtCaret}
                  onCollapse={() => setSidebarPanel(null)}
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
              <Show when={sidebarPanel() === "files"}>
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
                />
              </Show>
            </Suspense>
          </div>
          <div
            class="resizer-x"
            onMouseDown={sidebarResize.beginResize}
            title="Drag to resize"
          />
          <Show when={sidebarResize.previewPx() !== null}>
            <div
              class="pointer-events-none fixed top-0 bottom-0 w-px bg-[var(--ui-accent)] opacity-70 z-[70]"
              style={{ left: `${(sidebarResize.previewPx() ?? 0) + 44}px` }}
            />
          </Show>
        </Show>

        <div class="flex flex-1 flex-col min-h-0 min-w-0">
          <div class="flex flex-1 flex-col min-h-0" style={{ "background-color": "var(--ui-bg)", "background-image": "radial-gradient(ellipse 80% 50% at 50% 0%, var(--ui-selection), rgba(255,255,255,0))" }}>
            <SessionTabs
              leadingInsetPx={sidebarPanel() === null ? 36 : 0}
              sessions={sessionTabs()}
              activeSessionId={activeSessionId}
              setActiveSessionId={setActiveSessionId}
              onNewSession={newSession}
              onCloseSession={closeSession}
              updateSession={updateSession}
              onRefresh={() => { void refreshAssistants(); void refreshRoles(); void refreshSkills(); }}
              onToggleDrawer={() => setShowDrawer((v) => !v)}
              onToggleManagement={() => setShowManagement((v) => !v)}
            />

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
                  class="shrink-0 overflow-hidden"
                  style={{ height: `${Math.round(editorRatio() * splitContainerHeight())}px`, "border-bottom": "1px solid var(--ui-separator, var(--ui-border-strong))" }}
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

          <ChatInput
            input={input}
            setInput={setInput}
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
        </div>
      </div>

      <Suspense fallback={<div style={{ height: "22px" }} class="shrink-0 border-t theme-border theme-bg" />}>
        <StatusBar
          appSessionId={() => activeSession()?.id}
          cwd={() => activeSession()?.cwd ?? null}
          gitStatus={gitStatus}
          onOpenGit={() => setSidebarPanel("git")}
        />
      </Suspense>

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
            fetchRoleConfig={fetchRoleConfig}
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
            updateSession={updateSession}
            refreshRoles={refreshRoles}
            fetchRoleConfig={fetchRoleConfig}
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
