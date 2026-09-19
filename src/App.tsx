import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy, onCleanup, onMount } from "solid-js";

import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import { now, DEFAULT_ROLE_ALIAS } from "./components/types";
import type { AppSession, RuntimeCapabilities } from "./components/types";
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
  initialLeftSidebarOpen,
  initialRightDockWidth,
  initialRightPanel,
  type LeftDockPanel,
  type RightDockPanel,
} from "./lib/layoutTokens";
import ProjectSessionSidebar from "./components/sidebar/ProjectSessionSidebar";
import { hasConversationContent } from "./lib/conversationHelpers";

import { useSessionManager } from "./hooks/useSessionManager";
import { useStreamEngine } from "./hooks/useStreamEngine";
import { useAgentContext } from "./hooks/useAgentContext";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useAcpEventListeners } from "./hooks/useAcpEventListeners";
import { useCompletions } from "./hooks/useCompletions";
import { useMessageSend } from "./hooks/useMessageSend";
import { useInputHistory } from "./hooks/useInputHistory";
import { uniqueName, makeDefaultSession, makeDraftSession } from "./lib/sessionHelpers";
import { createSessionEventBuffer } from "./lib/sessionEventBuffer";
import { appSessionApi, assistantApi, projectAgentApi } from "./lib/tauriApi";
import { createCommandUiRegistry } from "./lib/commandUi/registry";
import type { PopupSelectSpec } from "./lib/commandUi/contract";
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
import { openPreviewTab } from "./lib/previewTabs";
import { destroySessionTerminal, updateTerminalThemes } from "./lib/terminalRuntime";
import { useProjects } from "./hooks/useProjects";
import ProjectModal from "./components/ProjectModal";
import ImportSessionsModal from "./components/ImportSessionsModal";
import type { Project, RawSession } from "./lib/tauriApi";

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
  const [showProjectModal, setShowProjectModal] = createSignal(false);
  const [importProject, setImportProject] = createSignal<Project | null>(null);

  const {
    projects,
    currentProject,
    selectProject,
    createProject,
    deleteProject,
    refreshProjects,
  } = useProjects(showToast);

  const [rightDockOpen, setRightDockOpenInternal] = createSignal(initialRightDockOpen());
  const [rightDockPanel, setRightDockPanelInternal] = createSignal<RightDockPanel | null>(
    initialRightDockOpen() ? (initialRightPanel() ?? "git") : null,
  );
  const [leftSidebarOpen, setLeftSidebarOpenInternal] = createSignal(initialLeftSidebarOpen());
  const toggleLeftSidebar = () => {
    const next = !leftSidebarOpen();
    setLeftSidebarOpenInternal(next);
    try { window.localStorage.setItem(LAYOUT_STORAGE.leftSidebarOpen, next ? "1" : "0"); } catch { /* ignore */ }
  };
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

  const sessionManager = useSessionManager(showToast);
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
    acceptingStreams,
    appendStream, appendThought,
    dropStream,
    normalizeToolLocations,
    scheduleCheckpoint,
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

  const activeRuntimeCapabilities = createMemo<RuntimeCapabilities | undefined>(() => {
    const session = activeSession();
    const runtime = assistants().find((assistant) =>
      (session?.runtimeProfileId && assistant.profileId === session.runtimeProfileId) ||
      (!session?.runtimeProfileId && assistant.key === session?.runtimeKind),
    );
    return runtime?.capabilities as RuntimeCapabilities | undefined;
  });

  const activeInputDelivery = createMemo(() => {
    const session = activeSession();
    const runtime = assistants().find((assistant) =>
      (session?.runtimeProfileId && assistant.profileId === session.runtimeProfileId) ||
      (!session?.runtimeProfileId && assistant.key === session?.runtimeKind),
    );
    return runtime?.inputDelivery;
  });

  let sendRawFn: ((text: string, silent?: boolean) => Promise<boolean>) | undefined;

  const commandRegistry = createCommandUiRegistry({
    activeSession,
    patchActiveSession,
    roles,
    resetActiveAgentContext,
    toggleRightDock,
    showToast: (message, tone) =>
      showToast(message, tone === "danger" || tone === "warning" ? "error" : tone === "success" ? "info" : tone),
    pushMessage: sessionManager.pushMessage,
    sendRaw: (t, s) => {
      if (sendRawFn) void sendRawFn(t, s);
      return Promise.resolve();
    },
    prewarmRoleConfig: assistantApi.prewarmRoleConfig,
  });

  const [popupSelectState, setPopupSelectState] = createSignal<{
    open: boolean;
    commandName: string;
    spec: PopupSelectSpec | null;
  }>({
    open: false,
    commandName: "",
    spec: null,
  });

  const handleTriggerCommandUi = (commandName: string): boolean => {
    const ui = commandRegistry.getCommandUi(commandName);
    if (!ui) return false;
    const s = activeSession();
    if (!s) return false;
    if (ui.kind === "action") {
      void ui.run(s);
      return true;
    }
    if (ui.kind === "popupSelect") {
      completions?.closeSlashMenu?.();
      setPopupSelectState({
        open: true,
        commandName,
        spec: ui,
      });
      return true;
    }
    return false;
  };

  const handleClosePopupSelect = (focusComposer = true) => {
    setPopupSelectState({ open: false, commandName: "", spec: null });
    if (focusComposer) {
      queueMicrotask(() => {
        richInputEl?.focus();
      });
    }
  };

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
    handleTriggerCommandUi,
    commandRegistry.getCommandList,
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

  // The single owner of the agent's option catalog (models / effort / toggles). It is a pure
  // function of (session, persona, engine), so it is pulled here whenever that triple moves
  // rather than pushed from each switch handler — every push site needed its own staleness
  // check, and the one that was missing is how a persona's model list kept showing another
  // persona's engine. A response is applied only if its triple is still the current one.
  createEffect(() => {
    const sid = activeSessionId();
    const session = activeSession();
    const role = session?.activeRole;
    const runtime = session?.runtimeKind;
    if (!sid || !role || !runtime) return;
    void fetchConfigOptions(runtime, role).then((opts) => {
      const current = activeSession();
      if (
        activeSessionId() === sid &&
        current?.activeRole === role &&
        current?.runtimeKind === runtime
      ) {
        patchActiveSession({ discoveredConfigOptions: opts });
      }
    });
  });

  const { bootstrapApp } = useAppBootstrap({
    setSessions,
    setActiveSessionId: (id) => setActiveSessionId(id),
    assistants,
    roles,
    currentProjectId: () => currentProject()?.id,
    refreshAssistants,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
    showToast,
  });

  const handleSelectProject = async (proj: Project, targetSessionId?: string) => {
    selectProject(proj);
    try {
      const raw = await appSessionApi.list();
      let loaded = raw.map((r) => {
        const s = makeDefaultSession(r.title);
        s.id = r.id;
        if (r.activeRole) s.activeRole = r.activeRole;
        if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
        if (r.runtimeProfileId !== undefined) s.runtimeProfileId = r.runtimeProfileId;
        if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
        if (r.projectId !== undefined) s.projectId = r.projectId ?? null;
        s.messages = r.messages ?? [];
        return s;
      });
      const projectSessions = loaded.filter((s) => s.projectId === proj.id);
      await Promise.all(loaded.map(async (session, index) => {
        if (!session.persisted) return;
        try {
          const inbox = await appSessionApi.listInbox(session.id);
          loaded[index].queuedItems = inbox.map((item) => ({
            id: item.id,
            text: item.text,
            attachments: item.attachments ?? [],
            roleName: item.roleName,
            delivery: item.delivery === "nextStep" ? "nextStep" : "nextTurn",
            createdAt: item.createdAt,
            status: "queued" as const,
          }));
        } catch {
          // Keep project switching available if restoring a legacy database inbox fails.
        }
      }));
      let nextActiveId: string;
      if (projectSessions.length === 0) {
        // No DB row until the user actually sends something — see `ensureSessionPersisted`.
        const availableAssistant = await preferredAssistantForProject(proj.id);
        const s = makeDraftSession("Session_1", {
          projectId: proj.id,
          cwd: proj.rootPath,
          runtimeKind: availableAssistant?.key ?? null,
          runtimeProfileId: availableAssistant?.profileId ?? null,
        });
        loaded = [...loaded, s];
        nextActiveId = s.id;
      } else {
        const target = targetSessionId ? projectSessions.find((s) => s.id === targetSessionId) : null;
        nextActiveId = target ? target.id : projectSessions[0].id;
      }
      setSessions(loaded);
      setActiveSessionId(nextActiveId);
      await refreshRoles(proj.id);
    } catch (e) {
      showToast(`Failed to switch project: ${String(e)}`);
    }
  };

  const openImportSessions = () => {
    const project = currentProject();
    if (!project) {
      showToast("Select a project before importing CLI sessions.", "error");
      return;
    }
    // Kobalte closes the project menu after its selection callback. Deferring the dialog by a
    // frame keeps the two modal focus lifecycles separate, so the picker reliably appears.
    window.requestAnimationFrame(() => setImportProject(project));
  };

  /**
   * Merge imported rows into the store. Replacing the store with a project-scoped list drops
   * every other project's sessions, and omitting projectId strands the new ones outside the
   * project group in the sidebar.
   */
  const mergeImportedSessions = (raw: RawSession[]) => {
    if (raw.length === 0) return;
    const existing = new Set(sessions.map((s) => s.id));
    const fresh = raw
      .filter((r) => !existing.has(r.id))
      .map((r) => {
        const s = makeDefaultSession(r.title);
        s.id = r.id;
        if (r.activeRole) s.activeRole = r.activeRole;
        if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
        if (r.runtimeProfileId !== undefined) s.runtimeProfileId = r.runtimeProfileId;
        if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
        s.projectId = r.projectId ?? null;
        s.messages = r.messages ?? [];
        return s;
      });
    if (fresh.length > 0) setSessions((prev) => [...prev, ...fresh]);
    const focus = fresh[0]?.id ?? raw[0].id;
    if (focus) setActiveSessionId(focus);
    showToast(`Imported ${raw.length} session(s)`, "info");
  };

  const { sendRaw, cancelCurrentRun, sendQueuedNow } = useMessageSend({
    sessionManager,
    streamEngine,
    agentContext,
    closeMentionMenu,
    closeSlashMenu,
    showToast,
    clearSessionStream,
    inputDelivery: activeInputDelivery,
  });
  sendRawFn = sendRaw;

  const inputHistory = useInputHistory(setInput);

  const chatActiveRole = createMemo(() => activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS);
  const chatSubmitting = createMemo(() => activeSession()?.submitting ?? false);
  const chatQueuedCount = createMemo(() =>
    activeSession()?.queuedItems.filter((item) => item.status === "queued").length ?? 0,
  );

  const handlePasteImage = (items: DataTransferItemList, currentNodes: RichNode[]) => {
    // Provider capability gate: attachments=false profiles (e.g. native agy)
    // cannot accept image input; refuse instead of silently dropping content
    // downstream.
    const session = activeSession();
    const profileId = session?.runtimeProfileId ?? null;
    const runtimeKind = session?.runtimeKind ?? null;
    const profile = assistants().find(
      (a) => (profileId && a.profileId === profileId) || (!profileId && a.key === runtimeKind),
    );
    if (profile && profile.capabilities.attachments === false) {
      showToast(`${profile.label} does not support image attachments for this session.`);
      return;
    }
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

    if (text.startsWith("/")) {
      const parts = text.slice(1).split(/\s+/);
      const cmdName = parts[0];
      const isBare = parts.length === 1;
      if (isBare && handleTriggerCommandUi(cmdName)) {
        setRichNodes([]);
        setRichCaretOffset(0);
        return;
      }
    }

    if (activeSession()?.submitting || activeSession()?.turnPhase === "cancelling" || activeSession()?.turnPhase === "sending") {
      setRichNodes([]);
      setRichCaretOffset(0);
      const sid = activeSessionId();
      if (sid) {
        const qidx = getSessionIndex(sid);
        const queuedText = text;
        const queuedAttachments = imageNodes.map((n) => n.img);
        const optimisticId = `pending-${now()}-${Math.random().toString(36).slice(2)}`;
        if (qidx !== -1) {
          setSessions(qidx, "queuedItems", (prev) => [...prev, {
            id: optimisticId,
            clientId: optimisticId,
            text: queuedText,
            attachments: queuedAttachments,
            roleName: activeSession()?.activeRole ?? null,
            delivery: "nextTurn" as const,
            createdAt: now(),
            status: "queued" as const,
          }]);
        }
        void appSessionApi.enqueueInbox(
          sid,
          queuedText,
          activeSession()?.activeRole ?? null,
          "nextTurn",
          queuedAttachments,
        ).then((saved) => {
          const current = getSessionIndex(sid);
          if (current === -1) {
            void appSessionApi.removeInbox(saved.id).catch(() => {});
            return;
          }
          const stillQueued = sessions[current]?.queuedItems.some(
            (item) => item.clientId === optimisticId || item.id === optimisticId,
          );
          if (!stillQueued) {
            void appSessionApi.removeInbox(saved.id).catch(() => {});
            return;
          }
          setSessions(current, "queuedItems", (prev) => prev.map((item) => item.clientId === optimisticId || item.id === optimisticId
            ? { ...item, id: saved.id, clientId: optimisticId, createdAt: saved.createdAt }
            : item));
        }).catch((error) => {
          const current = getSessionIndex(sid);
          if (current !== -1) {
          setSessions(current, "queuedItems", (prev) => prev.filter(
            (item) => item.id !== optimisticId && item.clientId !== optimisticId,
          ));
          }
          showToast(`Could not persist queued message: ${String(error)}`, "error");
        });
        scheduleScrollToBottom();
      }
      return;
    }
    const currentRole = roles().find((r) => r.roleName === (activeSession()?.activeRole ?? DEFAULT_ROLE_ALIAS));
    let effectiveRuntimeKind = activeSession()?.runtimeKind ?? currentRole?.runtimeKind ?? null;
    if (!effectiveRuntimeKind) {
      const availableAssistant = assistants().find((a) => a.available);
      if (availableAssistant) {
        effectiveRuntimeKind = availableAssistant.key;
        patchActiveSession({
          runtimeKind: availableAssistant.key,
          runtimeProfileId: availableAssistant.profileId,
        });
        const sid = activeSessionId();
        if (sid) {
          void appSessionApi.update(sid, {
            runtimeKind: availableAssistant.key,
            runtimeProfileId: availableAssistant.profileId,
          }).catch(() => {});
        }
      }
    }
    if (!effectiveRuntimeKind && !text.startsWith("/app_")) {
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

  const newSession = async (targetProjectId?: string) => {
    const targetProj = targetProjectId
      ? projects().find((p) => p.id === targetProjectId) ?? currentProject()
      : currentProject();
    const pid = targetProj?.id;
    if (targetProj && targetProj.id !== currentProject()?.id) {
      await handleSelectProject(targetProj);
    }
    // Draft only — no DB row until the user sends something (see `ensureSessionPersisted`),
    // so clicking "+ New Session" a few times while browsing never litters the DB.
    const availableAssistant = await preferredAssistantForProject(pid ?? null);
    const projectPath = targetProj?.rootPath ?? null;
    const targetSessions = pid ? sessions.filter((s) => s.projectId === pid) : sessions;
    const title = uniqueName("Session_1", targetSessions.map((s) => s.title));
    const s = makeDraftSession(title, {
      projectId: pid ?? null,
      cwd: projectPath,
      runtimeKind: availableAssistant?.key ?? null,
      runtimeProfileId: availableAssistant?.profileId ?? null,
    });
    setSessions(sessions.length, s);
    setActiveSessionId(s.id);
  };

  /**
   * Engine for a new session: the one this project pinned for the persona it will open with,
   * then that persona's own default, then whatever is available. Falling back straight to
   * "first available" silently resets the user's choice on every new session.
   */
  const preferredAssistantForProject = async (projectId: string | null, roleName?: string) => {
    const available = assistants().filter((a) => a.available);
    const role = roleName ?? DEFAULT_ROLE_ALIAS;
    const pinned = await projectAgentApi
      .getConfig(projectId, role)
      .then((c) => c.runtimeKind)
      .catch(() => null);
    const personaDefault = roles().find((r) => r.roleName === role)?.runtimeKind ?? null;
    return (
      available.find((a) => a.key === pinned) ??
      available.find((a) => a.key === personaDefault) ??
      available[0] ??
      null
    );
  };

  const closeSession = (id: string) => {
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

  const composerToolPanel = createMemo<LeftDockPanel | null>(() => rightDockPanel());
  const openWorkspacePanel = (panel: LeftDockPanel) => {
    setRightDockOpen(true);
    setRightDockPanel(panel === "commit" ? "git" : panel);
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


  useKeyboardShortcuts({
    newSession,
    openSettings: () => (showSettings() ? setShowSettings(false) : openSettings("general")),
    toggleManagement: () => openSettings("archived"),
    toggleRightDock,
    toggleLeftSidebar,
    openWorkspacePanel,
  });

  const composerBlock = (layout: "empty" | "active") => (
    <ChatInput
      layout={layout}
      activeSession={activeSession}
      popupSelectOpen={() => popupSelectState().open}
      popupSelectCommandName={() => popupSelectState().commandName}
      popupSelectSpec={() => popupSelectState().spec}
      onClosePopupSelect={handleClosePopupSelect}
      onTriggerCommandUi={handleTriggerCommandUi}
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
          activeToolPanel={composerToolPanel}
          onOpenToolPanel={openWorkspacePanel}
          onCancelRun={() => { void cancelCurrentRun(); }}
          onRunAction={runToolbarAction}
          onRefreshGit={refetchGitStatus}
          onManagePersonas={() => openSettings("roles")}
          onSelectAgent={(runtimeKind, profileId) => {
            // Switching agent keeps the persona; only the engine underneath changes.
            // The catalog for the new engine is loaded by syncConfigOptions' effect.
            patchActiveSession({ runtimeKind, runtimeProfileId: profileId, discoveredConfigOptions: [] });
            const sid = activeSessionId();
            if (sid) {
              void appSessionApi.update(sid, { runtimeKind, runtimeProfileId: profileId }).catch(() => {});
            }
          }}
          onSelectRole={(roleName) => {
            const role = roles().find((r) => r.roleName === roleName);
            const fallbackRuntime = assistants().find((a) => a.available);
            // A persona owns its engine: switching persona must visibly switch the CLI, or
            // the picker shows one persona's name over another's engine. The project's pin
            // for this persona is applied right after, by AgentPicker's load effect.
            const nextRuntimeKind =
              role?.runtimeKind ?? activeSession()?.runtimeKind ?? fallbackRuntime?.key ?? null;
            const nextProfileId =
              role?.runtimeProfileId ?? activeSession()?.runtimeProfileId ?? fallbackRuntime?.profileId ?? null;
            patchActiveSession({
              activeRole: roleName,
              runtimeKind: nextRuntimeKind,
              runtimeProfileId: nextProfileId,
              discoveredConfigOptions: [],
            });
            const sid = activeSessionId();
            if (sid) {
              void appSessionApi.update(sid, {
                activeRole: roleName,
                runtimeKind: nextRuntimeKind,
                runtimeProfileId: nextProfileId,
              }).catch(() => {});
            }
          }}
        />
      }
    />
  );

  onMount(() => {
    const handlers: Array<() => void> = [];
    let startupRaf: number | null = null;
    startupRaf = window.requestAnimationFrame(async () => {
      startupRaf = null;
      await refreshProjects();
      await bootstrapApp();
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
      onSessionDeltaLine: (sid, line, roleName) => sessionEventBuffer.push(sid, line, roleName),
      updateSession,
      mutateSession,
      appendThought,
      normalizeToolLocations,
      parseAgentCommands,
      normalizeRuntimeKey,
      roles,
      commandCacheKey,
      scheduleScrollToBottom,
      scheduleCheckpoint,
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

  const duplicateSession = async (session: AppSession) => {
    try {
      const newTitle = `${session.title}_copy`;
      const created = await appSessionApi.create(
        newTitle,
        session.projectId ?? undefined,
        session.runtimeKind ?? undefined,
        session.runtimeProfileId ?? undefined,
      );
      const s = makeDefaultSession(created.title || newTitle);
      s.id = created.id;
      s.projectId = session.projectId ?? null;
      s.runtimeKind = session.runtimeKind ?? null;
      s.runtimeProfileId = session.runtimeProfileId ?? null;
      s.cwd = session.cwd ?? null;
      s.activeRole = session.activeRole;
      setSessions((prev) => [s, ...prev]);
      setActiveSessionId(s.id);
      showToast(`Duplicated session '${s.title}'`, "info");
    } catch (e) {
      showToast(`Failed to duplicate session: ${String(e)}`, "error");
    }
  };

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
            onRestoreSession={(id, title, activeRole, runtimeKind, runtimeProfileId, cwd) => {
              const existing = getSessionIndex(id) !== -1;
              if (!existing) {
                const s = makeDefaultSession(title);
                s.id = id;
                s.activeRole = activeRole;
                s.runtimeKind = runtimeKind;
                s.runtimeProfileId = runtimeProfileId;
                s.cwd = cwd;
                setSessions(sessions.length, s);
              }
              setActiveSessionId(id);
              setShowSettings(false);
            }}
            onBack={() => setShowSettings(false)}
            showToast={showToast}
            currentProject={currentProject}
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
      leftSidebar={
        <Show when={leftSidebarOpen()}>
          <ProjectSessionSidebar
            projects={projects}
            currentProject={currentProject}
            sessions={() => sessions}
            activeSessionId={activeSessionId}
            onSelectProject={handleSelectProject}
            onSelectSession={async (sessionId, project) => {
              if (project && project.id !== currentProject()?.id) {
                await handleSelectProject(project, sessionId);
              } else {
                setActiveSessionId(sessionId);
              }
            }}
            onNewSession={(projectId) => { void newSession(projectId); }}
            onCloseSession={closeSession}
            onDuplicateSession={duplicateSession}
            onDeleteProject={deleteProject}
            onOpenAddProject={() => setShowProjectModal(true)}
            onOpenSettings={(tab) => openSettings((tab ?? "general") as SettingsTab)}
            onToggleSidebar={toggleLeftSidebar}
            updateSession={updateSession}
          />
        </Show>
      }
      sessionTopbar={
        <SessionTopbar
          sessions={sessions}
          activeSessionId={activeSessionId}
          setActiveSessionId={setActiveSessionId}
          updateSession={updateSession}
          onNewSession={() => { void newSession(); }}
          onCloseSession={closeSession}
          onOpenSettings={openSettings}
          onToggleRightDock={toggleRightDock}
          rightDockOpen={rightDockOpen}
          leftSidebarOpen={leftSidebarOpen}
          onToggleLeftSidebar={toggleLeftSidebar}
          currentProject={currentProject}
          projects={projects}
          onSelectProject={handleSelectProject}
          onSelectSession={async (sid, proj) => {
            if (proj && proj.id !== currentProject()?.id) {
              await handleSelectProject(proj, sid);
            } else {
              setActiveSessionId(sid);
            }
          }}
          onOpenAddProject={() => setShowProjectModal(true)}
          onImportSessions={openImportSessions}
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
              runtimeCapabilities={activeRuntimeCapabilities}
              inputDelivery={activeInputDelivery}
              patchActiveSession={patchActiveSession}
              onRemoveQueuedMessage={(itemId) => {
                const sid = activeSessionId();
                if (!sid) return;
                const idx = getSessionIndex(sid);
                if (idx === -1) return;
                const item = activeSession()?.queuedItems.find((queued) => queued.id === itemId || queued.clientId === itemId);
                setSessions(idx, "queuedItems", (prev) => prev.filter((queued) => queued.id !== itemId && queued.clientId !== itemId));
                if (item && !item.id.startsWith("pending-")) void appSessionApi.removeInbox(item.id).catch(() => {});
              }}
              onSendQueuedNow={() => { void sendQueuedNow(); }}
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
        <>
          <ImportSessionsModal
            open={importProject() !== null}
            project={importProject()}
            onClose={() => setImportProject(null)}
            onImported={mergeImportedSessions}
          />
          <ProjectModal
            open={showProjectModal()}
            onClose={() => setShowProjectModal(false)}
            onCreateProject={async (name, rootPath) => {
              const proj = await createProject(name, rootPath);
              if (proj) {
                await handleSelectProject(proj);
                setShowProjectModal(false);
              }
            }}
          />
          <div class="jockey-toast-stack fixed bottom-4 right-4 flex flex-col gap-2 pointer-events-none">
            <For each={toasts()}>
              {(t) => (
                <div class="jockey-toast pointer-events-auto" classList={{ "is-info": t.severity === "info", "is-danger": t.severity !== "info" }}>
                  {t.message}
                </div>
              )}
            </For>
          </div>
        </>
      }
    />
  );
}
