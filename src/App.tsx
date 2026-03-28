import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, Suspense, createSignal, lazy, onCleanup, onMount } from "solid-js";
import { produce } from "solid-js/store";

import SessionTabs from "./components/SessionTabs";
import MessageWindow from "./components/MessageWindow";
import ChatInput from "./components/ChatInput";
import type {
  Role, AppPlanEntry,
  AcpStreamEvent, AssistantChatResponse, AcpDeltaEvent, SessionUpdateEvent, WorkflowStateEvent,
  AppMessage, AppSession, AppToolCall,
} from "./components/types";
import {
  now, DEFAULT_BACKEND_ROLE, DEFAULT_ROLE_ALIAS,
} from "./components/types";

import { useSessionManager } from "./hooks/useSessionManager";
import { useStreamEngine } from "./hooks/useStreamEngine";
import { useAgentContext } from "./hooks/useAgentContext";
import { useCompletions } from "./hooks/useCompletions";
import {
  uniqueName, normalizeSessionTitle, isDefaultSessionTitle, makeDefaultSession,
} from "./lib/sessionHelpers";

const ConfigDrawer = lazy(() => import("./components/ConfigDrawer"));
const ManagementPanel = lazy(() => import("./components/ManagementPanel"));

export default function App() {
  type Toast = { id: number; message: string; severity?: "error" | "info" };
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  let toastSeq = 0;
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
    listRefMap, scheduleScrollToBottom,
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
    isCustomRole, activeBackendRole, runtimeForRole,
    refreshRoles, refreshSkills,
    fetchConfigOptions,
    parseAgentCommands,
    fetchAndCacheAgentCommands,
    setPreferredAssistant, refreshAssistants,
    resetActiveAgentContext,
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

  const queuedInputsFor = (sid: string | null): string[] => {
    if (!sid) return [];
    const s = sessions.find((x) => x.id === sid);
    return s?.queuedMessages ?? [];
  };

  let inputHistory: string[] = [];
  let historyIndex = -1;
  let historySavedInput = "";
  const HISTORY_MAX = 200;

  const runNextQueued = () => {
    const s = activeSession();
    if (s?.submitting) return;
    const sid = activeSessionId();
    const queue = queuedInputsFor(sid);
    if (queue.length === 0) return;
    if (sid) {
      setSessions((ss) => ss.id === sid, "queuedMessages", []);
    }
    const merged = queue.map((q) => q.trim()).filter(Boolean).join("\n");
    if (!merged) return;
    if (queue.length > 1) {
      pushMessage("event", `queued messages merged: ${queue.length}`);
    }
    void sendRaw(merged);
  };

  const cancelCurrentRun = async () => {
    await cancelCurrentRunBase(runNextQueued);
  };

  const sendRaw = async (text: string, silent = false) => {
    const runToken = bumpRunToken();
    const originSessionId = activeSessionId();
    closeMentionMenu();
    closeSlashMenu();

    const patchOriginSession = (patch: Partial<AppSession>) => {
      if (originSessionId) updateSession(originSessionId, patch);
    };

    const s = activeSession();
    let sendRoleLabel = s?.activeRole ?? DEFAULT_ROLE_ALIAS;
    let effectiveRole = s?.activeRole ?? DEFAULT_ROLE_ALIAS;
    const isCommand = text.startsWith("/");
    let inRoleContext = effectiveRole !== DEFAULT_ROLE_ALIAS && effectiveRole !== DEFAULT_BACKEND_ROLE;
    const isUnionAiCommand = text.startsWith("/app_");
    let routedText = text;
    const roleExists = (name: string) => roles().some((r) => r.roleName === name);

    if (!isCommand) {
      const mentionMatch = text.match(/^@(\S+)/);
      if (mentionMatch) {
        const rawTarget = mentionMatch[1];
        const target = rawTarget.startsWith("role:") ? rawTarget.slice(5) : rawTarget;
        if (
          target === "assistant" ||
          target === DEFAULT_ROLE_ALIAS ||
          target === DEFAULT_BACKEND_ROLE
        ) {
          patchActiveSession({ activeRole: DEFAULT_ROLE_ALIAS });
          sendRoleLabel = DEFAULT_ROLE_ALIAS;
          effectiveRole = DEFAULT_ROLE_ALIAS;
          inRoleContext = false;
          routedText = text.replace(/^@\S+\s*/, "").trim();
        } else {
          if (!roles().some((r) => r.roleName === target)) {
            pushMessage("event", `role not found: ${target}`);
            return;
          }
          patchActiveSession({ activeRole: target, discoveredConfigOptions: [] });
          sendRoleLabel = target;
          effectiveRole = target;
          inRoleContext = true;
          const targetRole = roles().find((r) => r.roleName === target);
          if (targetRole) {
            void fetchConfigOptions(targetRole.runtimeKind, targetRole.roleName).then((opts) => patchActiveSession({ discoveredConfigOptions: opts }));
            fetchAndCacheAgentCommands(targetRole.runtimeKind, targetRole.roleName);
          }
          routedText = text.replace(/^@\S+\s*/, "").trim();
          if (!routedText) return;
        }
      } else if (isCustomRole() && !text.startsWith("@")) {
        if (!roleExists(effectiveRole)) {
          pushMessage("event", `active role not found: ${effectiveRole}`);
          return;
        }
        routedText = `@${effectiveRole} ${text}`;
      }
    }

    const isRoleSlashCmd = isCommand && inRoleContext && !isUnionAiCommand;
    if (isRoleSlashCmd) {
      if (!roleExists(effectiveRole)) {
        pushMessage("event", `active role not found: ${effectiveRole}`);
        return;
      }
      routedText = `@${effectiveRole} ${text}`;
    }

    if (!silent) {
      pushMessage("user", text);
      const sid = activeSessionId();
      if (sid) {
        const sess = sessions.find((x) => x.id === sid);
        if (sess && isDefaultSessionTitle(sess.title) && sess.messages.filter((m) => m.roleName === "user").length === 0) {
          const cleaned = text.replace(/[@#][^\s]*/g, "").replace(/^\/\S+\s*/, "").trim();
          const words = cleaned.split(/\s+/);
          let autoTitle = "";
          for (const w of words) {
            if ((autoTitle + " " + w).trim().length > 40) break;
            autoTitle = (autoTitle + " " + w).trim();
          }
          if (!autoTitle) autoTitle = cleaned.slice(0, 30);
          autoTitle = normalizeSessionTitle(autoTitle);
          if (!autoTitle) autoTitle = `Session_${Date.now()}`;
          const existing = sessions.filter((x) => x.id !== sid).map((x) => x.title);
          autoTitle = uniqueName(autoTitle, existing);
          updateSession(sid, { title: autoTitle });
          void invoke("update_app_session", { id: sid, update: { title: autoTitle } }).catch(() => {});
        }
      }
    }
    patchOriginSession({ submitting: true, status: "running", agentState: undefined, thoughtText: "" });

    const isAgentCmd = isCommand && !inRoleContext && /^\/(plan|act|auto|cancel)\b/.test(text);
    if (isAgentCmd) {
      const role = activeBackendRole();
      const runtime = runtimeForRole(role);
      const cmd = text.split(/\s+/)[0].slice(1);
      try {
        if (cmd === "cancel") {
          if (runtime) await invoke("cancel_acp_session", { runtimeKind: runtime, roleName: role, appSessionId: activeSessionId() ?? "" });
          pushMessage("event", `cancelled ${role}`);
        } else {
          if (runtime) await invoke("set_acp_mode", { runtimeKind: runtime, roleName: role, modeId: cmd, appSessionId: activeSessionId() ?? "" });
          pushMessage("event", `${role} mode → ${cmd}`);
        }
      } catch (e) {
        pushMessage("event", String(e));
      } finally {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued();
      }
      return;
    }

    const appendOriginMessage = (msg: AppMessage) => {
      if (!originSessionId) return;
      appendMessageToSession(originSessionId, msg);
    };

    const startOriginStream = () => {
      if (originSessionId) acceptingStreams.add(originSessionId);
      const id = `stream-${now()}`;
      const row: AppMessage = { id, roleName: sendRoleLabel, text: "", at: now() };
      patchOriginSession({ streamingMessage: row, toolCalls: {}, streamSegments: [], currentPlan: null, pendingPermission: null, thoughtText: "" });
      scheduleScrollToBottom();
      return row;
    };

    const completeOriginStream = (finalReply?: string) => {
      const sid = originSessionId;
      if (!sid) return;
      finalizeSessionStream(sid, sendRoleLabel, finalReply);
    };

    const dropOriginStream = () => {
      patchOriginSession({ streamingMessage: null, thoughtText: "" });
      resetStreamState(originSessionId ?? undefined);
    };

    let streamStarted = false;
    if ((!isUnionAiCommand) && s?.runtimeKind) {
      startOriginStream();
      streamStarted = true;
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: {
          input: routedText,
          runtimeKind: s?.runtimeKind ?? null,
          appSessionId: originSessionId ?? null,
        }
      });
      if (runToken <= getCanceledRunToken()) return;
      if (res.runtimeKind) setPreferredAssistant(res.runtimeKind);
      if (text.startsWith("/app_role")) void refreshRoles();

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
      patchOriginSession({ submitting: false, status: "error" });
      if (originSessionId) acceptingStreams.delete(originSessionId);
      runNextQueued();
      return;
    } finally {
      if (originSessionId) acceptingStreams.delete(originSessionId);
      if (runToken <= getCanceledRunToken()) {
        patchOriginSession({ submitting: false, status: "idle" });
        runNextQueued();
        return;
      }
      patchOriginSession({ submitting: false, status: "done" });
      runNextQueued();
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
    void invoke<{ id: string }>("create_app_session", { title }).then((created) => {
      const s = makeDefaultSession(title);
      s.id = created.id;
      s.runtimeKind = availableAssistant;
      setSessions(sessions.length, s);
      setActiveSessionId(s.id);
      if (availableAssistant) {
        void invoke("update_app_session", {
          id: created.id,
          update: { runtimeKind: availableAssistant },
        }).catch(() => { });
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
    if (remaining.length === 0) { void invoke("delete_app_session", { id }).catch(() => {}); return; }
    setSessions(remaining);
    if (activeSessionId() === id) {
      const remaining = sessions.filter((s) => s.id !== id);
      if (remaining.length > 0) {
        setActiveSessionId(remaining[remaining.length - 1].id);
      }
    }
    void invoke("delete_app_session", { id }).catch(() => {});
  };

  onMount(() => {
    const handlers: UnlistenFn[] = [];
    let startupRaf: number | null = null;
    const boot = async () => {
      let loaded: AppSession[] = [];
      try {
        const raw = await invoke<Array<{ id: string; title: string; messages: AppMessage[]; activeRole?: string; runtimeKind?: string | null; cwd?: string | null }>>("list_app_sessions");
        loaded = raw.map((r) => {
          const s = makeDefaultSession(r.title);
          s.id = r.id;
          if (r.activeRole) s.activeRole = r.activeRole;
          if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
          if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
          s.messages = r.messages ?? [];
          return s;
        });
      } catch (e) {
        showToast(`Failed to restore sessions: ${String(e)}`);
      }

      if (loaded.length === 0) {
        try {
          const created = await invoke<{ id: string }>("create_app_session", { title: "Session_1" });
          const s = makeDefaultSession("Session_1");
          s.id = created.id;
          loaded = [s];
        } catch {
          loaded = [makeDefaultSession("Session_1")];
        }
      }

      setSessions(loaded);
      setActiveSessionId(loaded[0].id);

      await Promise.all([
        refreshAssistants(),
        refreshRoles(),
        refreshSkills(),
      ]);

      const availableAssistant = assistants().find((a) => a.available)?.key ?? null;

      sessions.forEach((s, i) => {
        if (!s.runtimeKind && availableAssistant) {
          setSessions(i, "runtimeKind", availableAssistant);
          void invoke("update_app_session", {
            id: s.id,
            update: { runtimeKind: availableAssistant },
          }).catch(() => { });
        }
      });

      pushMessage("system", "Welcome to UnionAI. Agent sessions are warming up in the background.");

      assistants().filter((a) => a.available).forEach((a) => {
        void fetchConfigOptions(a.key);
      });
    };
    startupRaf = window.requestAnimationFrame(() => {
      startupRaf = null;
      void boot();
    });

    void Promise.all([
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        const sid = ev.payload.appSessionId;
        if (!sid || !acceptingStreams.has(sid)) return;
        const sess = sessions.find((s) => s.id === sid);
        if (!sess?.streamingMessage) return;
        appendStream(sid, ev.payload.delta);
      }),
      listen<SessionUpdateEvent>("session/update", (ev) => {
        if (!ev.payload.delta) return;
        streamEngine.pendingSessionEvents.push(`[${ev.payload.roleName}] ${ev.payload.delta}`);
        scheduleSessionEventFlush();
      }),
      listen<WorkflowStateEvent>("workflow/state_changed", (ev) => {
        const p = ev.payload;
        pushMessage("event", `[workflow] ${p.status} ${p.activeRole ?? ""} ${p.message}`);
      }),
      listen<{ role: string; runtimeKind?: string; appSessionId?: string; event: AcpStreamEvent }>("acp/stream", (ev) => {
        const e = ev.payload.event;
        const sid = ev.payload.appSessionId;
        if (!sid || !acceptingStreams.has(sid)) return;
        const patchSession = (patch: Partial<AppSession>) => updateSession(sid, patch);
        switch (e.kind) {
          case "statusUpdate":
            if (e.text) patchSession({ agentState: e.text });
            break;
          case "thoughtDelta":
            if (e.text) {
              patchSession({ agentState: `Thinking: ${e.text.slice(0, 120)}` });
              if (sid) appendThought(sid, e.text);
            }
            break;
          case "toolCall":
            if (e.toolCallId) {
              const content = Array.isArray(e.content) ? e.content : undefined;
              const locations = normalizeToolLocations(e.locations as unknown[] | undefined);
              const tc: AppToolCall = {
                toolCallId: e.toolCallId!,
                title: e.title ?? "",
                kind: e.toolKind ?? "unknown",
                status: e.status ?? "pending",
                content,
                contentJson: content && content.length > 0 ? JSON.stringify(content, null, 2) : undefined,
                locations,
                rawInput: e.rawInput,
                rawOutput: e.rawOutput,
                rawInputJson: e.rawInput !== undefined ? JSON.stringify(e.rawInput, null, 2) : undefined,
                rawOutputJson: e.rawOutput !== undefined ? JSON.stringify(e.rawOutput, null, 2) : undefined,
              };
              setSessions((s) => s.id === sid, produce((s) => {
                s.toolCalls[e.toolCallId!] = tc;
                const existing = s.streamSegments.findIndex(seg => seg.kind === "tool" && seg.tc.toolCallId === e.toolCallId);
                if (existing >= 0) {
                  s.streamSegments[existing] = { kind: "tool" as const, tc };
                } else {
                  s.streamSegments.push({ kind: "tool" as const, tc });
                }
              }));
              patchSession({ agentState: `${e.toolKind ?? "tool"}: ${e.title ?? e.toolCallId}` });
              scheduleScrollToBottom();
            }
            break;
          case "toolCallUpdate":
            if (e.toolCallId) {
              setSessions((s) => s.id === sid, produce((s) => {
                const isNew = !s.toolCalls[e.toolCallId!];
                const existing = s.toolCalls[e.toolCallId!] ?? {
                  toolCallId: e.toolCallId!,
                  title: e.title ?? "",
                  kind: e.toolKind ?? "unknown",
                  status: "pending",
                };
                const newContent = Array.isArray(e.content) ? e.content : existing.content;
                const newLocations = normalizeToolLocations(e.locations as unknown[] | undefined) ?? existing.locations;
                const contentJson = newContent !== existing.content
                  ? (newContent && newContent.length > 0 ? JSON.stringify(newContent, null, 2) : undefined)
                  : existing.contentJson;
                const newRawInput = e.rawInput !== undefined ? e.rawInput : existing.rawInput;
                const newRawOutput = e.rawOutput !== undefined ? e.rawOutput : existing.rawOutput;
                const updated: AppToolCall = {
                  ...existing,
                  kind: e.toolKind ?? existing.kind,
                  status: e.status ?? existing.status,
                  title: e.title ?? existing.title,
                  content: newContent,
                  contentJson,
                  locations: newLocations,
                  rawInput: newRawInput,
                  rawOutput: newRawOutput,
                  rawInputJson: e.rawInput !== undefined ? JSON.stringify(newRawInput, null, 2) : existing.rawInputJson,
                  rawOutputJson: e.rawOutput !== undefined ? JSON.stringify(newRawOutput, null, 2) : existing.rawOutputJson,
                };
                s.toolCalls[e.toolCallId!] = updated;
                if (isNew) {
                  s.streamSegments.push({ kind: "tool", tc: updated });
                } else {
                  for (let i = s.streamSegments.length - 1; i >= 0; i--) {
                    const seg = s.streamSegments[i];
                    if (seg.kind === "tool" && seg.tc.toolCallId === e.toolCallId) {
                      s.streamSegments[i] = { kind: "tool", tc: updated };
                      break;
                    }
                  }
                }
              }));
              if (e.status || e.title) {
                patchSession({
                  agentState: `${e.toolKind ?? "tool"} ${e.status ?? "updated"}: ${e.title ?? e.toolCallId}`,
                });
              }
              scheduleScrollToBottom();
            }
            break;
          case "plan":
            if (e.entries) patchSession({ currentPlan: e.entries as AppPlanEntry[] });
            break;
          case "permissionRequest":
            if (e.requestId) {
              patchSession({
                pendingPermission: {
                  requestId: e.requestId,
                  title: e.title ?? "Permission Required",
                  description: e.description ?? null,
                  options: (e.options as Array<{ optionId: string; title?: string }>) ?? [],
                },
              });
            }
            break;
          case "modeUpdate":
            if (e.modeId) patchSession({ currentMode: e.modeId });
            break;
          case "availableModes":
            if (e.modes) patchSession({ agentModes: e.modes });
            if (e.current !== undefined) patchSession({ currentMode: e.current ?? null });
            break;
          case "availableCommands":
            if (e.commands) {
              const roleName = ev.payload.role;
              if (roleName) {
                const parsed = parseAgentCommands(e.commands as unknown[]);
                const runtimeKey = ev.payload.runtimeKind
                  || roles().find((r: Role) => r.roleName === roleName)?.runtimeKind
                  || "";
                const normalizedRuntime = runtimeKey ? normalizeRuntimeKey(runtimeKey) : "";
                if (!normalizedRuntime) break;
                const key = commandCacheKey(normalizedRuntime, roleName);
                setSessions((s) => s.id === sid, produce((s) => {
                  const next = new Map(s.agentCommands);
                  next.set(key, parsed);
                  s.agentCommands = next;
                }));
              }
            }
            break;
          default:
            break;
        }
      }),
    ]).then((hs) => handlers.push(...hs));

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
      if (streamEngine.sessionEventFlushTimer !== null) window.clearTimeout(streamEngine.sessionEventFlushTimer);
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
        onListMounted={(id, el) => { listRefMap.set(id, el); scheduleScrollToBottom(); }}
        onListUnmounted={(id) => { listRefMap.delete(id); }}
      />

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
            pushMessage={pushMessage}
            fetchConfigOptions={fetchConfigOptions}
            onOpenManagement={(tab, roleName) => {
              setManagementInitialTab(tab ?? "sessions");
              setManagementInitialRole(roleName);
              setShowManagement(true);
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
            <div class={`pointer-events-auto max-w-xs rounded-lg px-4 py-2 text-xs shadow-lg ${t.severity === "info" ? "bg-zinc-800 text-zinc-300" : "bg-rose-900/90 text-rose-200"}`}>
              {t.message}
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
