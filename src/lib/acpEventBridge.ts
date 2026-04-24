import type {
  AcpDeltaEvent,
  AcpStreamEvent,
  AppPlanEntry,
  AppSession,
  AppToolCall,
  Role,
  SessionUpdateEvent,
  TerminalEntry,
  WorkflowStateEvent,
} from "../components/types";

type TerminalInfo = { terminalId?: string; cwd?: string | null; label?: string } | undefined;
type TerminalOutput = { terminalId?: string; data?: string } | undefined;
type TerminalExit = { terminalId?: string; exitCode?: number; signal?: string | null } | undefined;

function readTerminalMeta(meta: unknown): {
  info: TerminalInfo;
  output: TerminalOutput;
  exit: TerminalExit;
} {
  if (!meta || typeof meta !== "object") {
    return { info: undefined, output: undefined, exit: undefined };
  }
  const m = meta as Record<string, unknown>;
  const info = (m.terminalInfo ?? m.terminal_info) as TerminalInfo;
  const output = (m.terminalOutput ?? m.terminal_output) as TerminalOutput;
  const exit = (m.terminalExit ?? m.terminal_exit) as TerminalExit;
  return { info, output, exit };
}

function applyTerminalMetaToSession(
  session: AppSession,
  event: AcpStreamEvent,
  tc: AppToolCall,
): void {
  const { info, output, exit } = readTerminalMeta(event.terminalMeta);
  // `terminal_info`: upsert entry and flush any buffered output chunks.
  if (info?.terminalId) {
    const tid = info.terminalId;
    const existing = session.terminals[tid];
    const entry: TerminalEntry = existing ?? {
      terminalId: tid,
      label: info.label ?? tc.title ?? null,
      cwd: info.cwd ?? null,
      output: "",
      exitStatus: null,
    };
    if (!existing) {
      const buffered = session.pendingTerminalOutput[tid];
      if (buffered && buffered.length > 0) {
        entry.output += buffered.join("");
        delete session.pendingTerminalOutput[tid];
      }
    }
    session.terminals[tid] = entry;
  }
  // `terminal_output`: append to entry or buffer if info hasn't arrived yet.
  if (output?.terminalId && typeof output.data === "string") {
    const tid = output.terminalId;
    const chunk = output.data;
    const entry = session.terminals[tid];
    if (entry) {
      entry.output += chunk;
    } else {
      const pending = session.pendingTerminalOutput[tid] ?? [];
      pending.push(chunk);
      session.pendingTerminalOutput[tid] = pending;
    }
  }
  // `terminal_exit`: set exit status on the entry (if registered).
  if (exit?.terminalId) {
    const entry = session.terminals[exit.terminalId];
    if (entry) {
      entry.exitStatus = {
        exitCode: exit.exitCode,
        signal: exit.signal ?? null,
      };
    }
  }
}

type BridgeDeps = {
  sid: string;
  roleName: string;
  runtimeKind?: string;
  event: AcpStreamEvent;
  patchSession: (sid: string, patch: Partial<AppSession>) => void;
  mutateSession: (sid: string, recipe: (s: AppSession) => void) => void;
  appendThought: (sid: string, text: string) => void;
  normalizeToolLocations: (
    raw: unknown[] | undefined,
  ) => Array<{ path: string; line?: number }> | undefined;
  parseAgentCommands: (
    raw: unknown[],
  ) => Array<{ name: string; description: string; hint?: string }>;
  normalizeRuntimeKey: (runtimeKey: string) => string;
  roles: () => Role[];
  commandCacheKey: (runtimeKey: string, roleName: string) => string;
  scheduleScrollToBottom: () => void;
};

export function toConnectionLostMessage(payload: {
  runtimeKey: string;
  roleName: string;
  reason?: string | null;
}): string {
  const suffix = payload.reason ? ` Reason: ${payload.reason}` : "";
  return `Agent ${payload.runtimeKey} (role: ${payload.roleName}) disconnected — will reconnect on next message.${suffix}`;
}

export function toWorkflowStateMessage(payload: WorkflowStateEvent): string {
  return `[workflow] ${payload.status} ${payload.activeRole ?? ""} ${payload.message}`;
}

export function toSessionDeltaMessage(payload: SessionUpdateEvent): string | null {
  if (!payload.delta) return null;
  return `[${payload.roleName}] ${payload.delta}`;
}

export function appendAcpDelta(
  payload: AcpDeltaEvent & { appSessionId?: string },
  acceptingStreams: Map<string, number>,
  sessions: AppSession[],
  appendStream: (sid: string, chunk: string) => void,
  getSessionIndex?: (id: string) => number,
): void {
  const sid = payload.appSessionId;
  if (!sid || !acceptingStreams.has(sid)) return;
  const idx = getSessionIndex ? getSessionIndex(sid) : sessions.findIndex((s) => s.id === sid);
  if (idx === -1) return;
  if (!sessions[idx]?.streamingMessage) return;
  appendStream(sid, payload.delta);
}

export function applyAcpStreamEvent(deps: BridgeDeps): void {
  const {
    sid,
    roleName,
    runtimeKind,
    event,
    patchSession,
    mutateSession,
    appendThought,
    normalizeToolLocations,
    parseAgentCommands,
    normalizeRuntimeKey,
    roles,
    commandCacheKey,
    scheduleScrollToBottom,
  } = deps;

  switch (event.kind) {
    case "statusUpdate":
      if (event.text) patchSession(sid, { agentState: event.text });
      break;
    case "thoughtDelta":
      if (event.text) {
        patchSession(sid, { agentState: `Thinking: ${event.text.slice(0, 120)}` });
        appendThought(sid, event.text);
      }
      break;
    case "toolCall":
      if (event.toolCallId) {
        const content = Array.isArray(event.content) ? event.content : undefined;
        const locations = normalizeToolLocations(event.locations as unknown[] | undefined);
        const tc: AppToolCall = {
          toolCallId: event.toolCallId,
          title: event.title ?? "",
          kind: event.toolKind ?? "unknown",
          status: event.status ?? "pending",
          content,
          contentJson:
            content && content.length > 0
              ? JSON.stringify(content, null, 2)
              : undefined,
          locations,
          rawInput: event.rawInput,
          rawOutput: event.rawOutput,
          terminalMeta: event.terminalMeta,
          rawInputJson:
            event.rawInput !== undefined
              ? JSON.stringify(event.rawInput, null, 2)
              : undefined,
          rawOutputJson:
            event.rawOutput !== undefined
              ? JSON.stringify(event.rawOutput, null, 2)
              : undefined,
        };
        mutateSession(sid, (s) => {
          s.toolCalls[event.toolCallId!] = tc;
          const existing = s.streamSegments.findIndex(
            (seg) => seg.kind === "tool" && seg.tc.toolCallId === event.toolCallId,
          );
          if (existing >= 0) {
            s.streamSegments[existing] = { kind: "tool", tc };
          } else {
            s.streamSegments.push({ kind: "tool", tc });
          }
          applyTerminalMetaToSession(s, event, tc);
        });
        patchSession(sid, {
          agentState: `${event.toolKind ?? "tool"}: ${event.title ?? event.toolCallId}`,
        });
        scheduleScrollToBottom();
      }
      break;
    case "toolCallUpdate":
      if (event.toolCallId) {
        mutateSession(sid, (s) => {
          const isNew = !s.toolCalls[event.toolCallId!];
          const existing = s.toolCalls[event.toolCallId!] ?? {
            toolCallId: event.toolCallId!,
            title: event.title ?? "",
            kind: event.toolKind ?? "unknown",
            status: "pending",
          };
          const newContent = Array.isArray(event.content)
            ? event.content
            : existing.content;
          const newLocations =
            normalizeToolLocations(event.locations as unknown[] | undefined) ??
            existing.locations;
          const contentJson =
            newContent !== existing.content
              ? newContent && newContent.length > 0
                ? JSON.stringify(newContent, null, 2)
                : undefined
              : existing.contentJson;
          const newRawInput =
            event.rawInput !== undefined ? event.rawInput : existing.rawInput;
          const newRawOutput =
            event.rawOutput !== undefined ? event.rawOutput : existing.rawOutput;
          const newTerminalMeta =
            event.terminalMeta !== undefined ? event.terminalMeta : existing.terminalMeta;
          const updated: AppToolCall = {
            ...existing,
            kind: event.toolKind ?? existing.kind,
            status: event.status ?? existing.status,
            title: event.title ?? existing.title,
            content: newContent,
            contentJson,
            locations: newLocations,
            rawInput: newRawInput,
            rawOutput: newRawOutput,
            terminalMeta: newTerminalMeta,
            rawInputJson:
              event.rawInput !== undefined
                ? JSON.stringify(newRawInput, null, 2)
                : existing.rawInputJson,
            rawOutputJson:
              event.rawOutput !== undefined
                ? JSON.stringify(newRawOutput, null, 2)
                : existing.rawOutputJson,
          };
          s.toolCalls[event.toolCallId!] = updated;
          if (isNew) {
            s.streamSegments.push({ kind: "tool", tc: updated });
          } else {
            for (let i = s.streamSegments.length - 1; i >= 0; i--) {
              const seg = s.streamSegments[i];
              if (seg.kind === "tool" && seg.tc.toolCallId === event.toolCallId) {
                s.streamSegments[i] = { kind: "tool", tc: updated };
                break;
              }
            }
          }
          applyTerminalMetaToSession(s, event, updated);
        });
        if (event.status || event.title) {
          patchSession(sid, {
            agentState: `${event.toolKind ?? "tool"} ${event.status ?? "updated"}: ${
              event.title ?? event.toolCallId
            }`,
          });
        }
        scheduleScrollToBottom();
      }
      break;
    case "plan":
      if (event.entries) patchSession(sid, { currentPlan: event.entries as AppPlanEntry[] });
      break;
    case "permissionRequest":
      if (event.requestId) {
        patchSession(sid, {
          pendingPermission: {
            requestId: event.requestId,
            title: event.title ?? "Permission Required",
            description: event.description ?? null,
            options:
              (event.options as Array<{ optionId: string; title?: string }>) ?? [],
          },
        });
      }
      break;
    case "permissionExpired":
      if (event.requestId) {
        mutateSession(sid, (s) => {
          if (s.pendingPermission?.requestId === event.requestId) {
            s.pendingPermission = null;
          }
        });
      }
      break;
    case "modeUpdate":
      if (event.modeId) patchSession(sid, { currentMode: event.modeId });
      break;
    case "availableModes":
      if (event.modes) patchSession(sid, { agentModes: event.modes });
      if (event.current !== undefined) {
        patchSession(sid, { currentMode: event.current ?? null });
      }
      break;
    case "availableCommands":
      if (event.commands && roleName) {
        const parsed = parseAgentCommands(event.commands as unknown[]);
        const runtimeKey =
          runtimeKind ||
          roles().find((r) => r.roleName === roleName)?.runtimeKind ||
          "";
        const normalizedRuntime = runtimeKey ? normalizeRuntimeKey(runtimeKey) : "";
        if (!normalizedRuntime) break;
        const key = commandCacheKey(normalizedRuntime, roleName);
        mutateSession(sid, (s) => {
          const next = new Map(s.agentCommands);
          next.set(key, parsed);
          s.agentCommands = next;
        });
      }
      break;
    case "sessionError":
      if (event.code && event.message) {
        patchSession(sid, {
          lastError: {
            code: event.code,
            message: event.message,
            retryable: event.retryable === true,
          },
        });
      }
      break;
    default:
      break;
  }
}
