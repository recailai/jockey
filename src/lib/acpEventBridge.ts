import { normalizePermissionOptions } from "./permissionOptions";
import type {
  AcpDeltaEvent,
  AcpStreamEvent,
  AppPermission,
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

function normalizeToolStatus(status: string | undefined): string | undefined {
  if (!status) return status;
  const normalized = status.replace(/[-_]/g, "").toLowerCase();
  if (normalized === "inprogress" || normalized === "running") return "running";
  if (normalized === "pending") return "pending";
  if (normalized === "completed" || normalized === "success") return "completed";
  if (normalized === "failed" || normalized === "failure" || normalized === "error") return "failure";
  if (normalized === "declined" || normalized === "cancelled" || normalized === "canceled") return "cancelled";
  return status;
}

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

type PendingOutput = { chunks: Map<string, string>; mutateSession: BridgeDeps["mutateSession"] };
const pendingOutput = new Map<string, PendingOutput>();
// toolCallId -> index in streamSegments. Segments are only appended or replaced in place, so an
// index stays valid; it is re-checked on use and rebuilt by a scan only when stale.
const segmentIndex = new Map<string, Map<string, number>>();
let outputFrame: number | null = null;

function scheduleOutputFlush() {
  if (outputFrame !== null) return;
  const run = () => { outputFrame = null; flushToolOutputDeltas(); };
  outputFrame = typeof requestAnimationFrame === "function"
    ? requestAnimationFrame(run)
    : (setTimeout(run, 16) as unknown as number);
}

function queueToolOutput(sid: string, toolCallId: string, delta: string, mutateSession: BridgeDeps["mutateSession"]) {
  let entry = pendingOutput.get(sid);
  if (!entry) {
    entry = { chunks: new Map(), mutateSession };
    pendingOutput.set(sid, entry);
  }
  entry.chunks.set(toolCallId, (entry.chunks.get(toolCallId) ?? "") + delta);
  scheduleOutputFlush();
}

/** Applies buffered tool output. Called before any other event for the session so ordering
 *  against tool updates and turn end is preserved. */
export function flushToolOutputDeltas(sid?: string): void {
  const sids = sid ? [sid] : [...pendingOutput.keys()];
  for (const id of sids) {
    const entry = pendingOutput.get(id);
    if (!entry) continue;
    pendingOutput.delete(id);
    entry.mutateSession(id, (s) => {
      let index = segmentIndex.get(id);
      if (!index) {
        index = new Map();
        segmentIndex.set(id, index);
      }
      for (const [toolCallId, delta] of entry.chunks) {
        const tc = s.toolCalls[toolCallId];
        if (!tc) continue;
        const next = { ...tc, outputLog: `${tc.outputLog ?? ""}${delta}` };
        s.toolCalls[toolCallId] = next;
        let at = index.get(toolCallId) ?? -1;
        const hit = (i: number) => {
          const seg = s.streamSegments[i];
          return seg?.kind === "tool" && seg.tc.toolCallId === toolCallId;
        };
        if (at < 0 || !hit(at)) {
          at = s.streamSegments.findIndex((_, i) => hit(i));
          if (at >= 0) index.set(toolCallId, at);
        }
        if (at >= 0) {
          const seg = s.streamSegments[at];
          s.streamSegments[at] = { kind: "tool", tc: next, roleName: seg.kind === "tool" ? seg.roleName : undefined };
        }
      }
    });
  }
}

export type BridgeDeps = {
  sid: string;
  roleName: string;
  runtimeKind?: string;
  event: AcpStreamEvent;
  patchSession: (sid: string, patch: Partial<AppSession>) => void;
  mutateSession: (sid: string, recipe: (s: AppSession) => void) => void;
  appendThought: (sid: string, text: string, roleName?: string) => void;
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

export function toSessionDeltaMessage(payload: SessionUpdateEvent): { text: string; roleName?: string } | null {
  if (!payload.delta) return null;
  return { text: payload.delta, roleName: payload.roleName || undefined };
}

export function appendAcpDelta(
  payload: AcpDeltaEvent & { appSessionId?: string },
  acceptingStreams: Map<string, number>,
  sessions: AppSession[],
  appendStream: (sid: string, chunk: string, roleName?: string) => void,
  getSessionIndex?: (id: string) => number,
): void {
  const sid = payload.appSessionId;
  if (!sid || !acceptingStreams.has(sid)) return;
  const idx = getSessionIndex ? getSessionIndex(sid) : sessions.findIndex((s) => s.id === sid);
  if (idx === -1) return;
  if (!sessions[idx]?.streamingMessage) return;
  if (payload.role && sessions[idx].streamingMessage && sessions[idx].streamingMessage!.roleName !== payload.role) {
    sessions[idx].streamingMessage!.roleName = payload.role;
  }
  appendStream(sid, payload.delta, payload.role);
}

export function applyAcpStreamEvent(deps: BridgeDeps): void {
  if (deps.event.kind !== "toolOutputDelta" && pendingOutput.has(deps.sid)) flushToolOutputDeltas(deps.sid);
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
      if (event.text) {
        patchSession(sid, { agentState: event.text });
      }
      break;
    case "thoughtDelta":
      if (event.text) {
        patchSession(sid, { agentState: `Thinking: ${event.text.slice(0, 120)}` });
        appendThought(sid, event.text, roleName);
      }
      break;
    case "toolCall":
      if (event.toolCallId) {
        const content = Array.isArray(event.content) ? event.content : undefined;
        const locations = normalizeToolLocations(event.locations as unknown[] | undefined);
        const tc: AppToolCall = {
          toolCallId: event.toolCallId,
          title: event.title ?? "",
          toolName: event.toolName,
          kind: event.toolKind ?? "unknown",
          status: normalizeToolStatus(event.status) ?? "pending",
          // Display text is derived on demand in ToolCallGroup; pretty-printing large payloads
          // on every update cost far more than it bought.
          content,
          locations,
          rawInput: event.rawInput ?? undefined,
          rawOutput: event.rawOutput ?? undefined,
          terminalMeta: event.terminalMeta,
          parentId: event.parentId ?? null,
          diff: event.diff,
          roleName,
        };
        mutateSession(sid, (s) => {
          s.toolCalls[event.toolCallId!] = tc;
          const existing = s.streamSegments.findIndex(
            (seg) => seg.kind === "tool" && seg.tc.toolCallId === event.toolCallId,
          );
          if (existing >= 0) {
            s.streamSegments[existing] = { kind: "tool", tc, roleName };
          } else {
            s.streamSegments.push({ kind: "tool", tc, roleName });
          }
          applyTerminalMetaToSession(s, event, tc);
        });
        patchSession(sid, {
          agentState: `${event.toolName ?? event.toolKind ?? "tool"}: ${event.title ?? event.toolCallId}`,
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
            toolName: event.toolName,
            kind: event.toolKind ?? "unknown",
            status: "pending",
          };
          const newContent = Array.isArray(event.content)
            ? event.content
            : existing.content;
          const newLocations =
            normalizeToolLocations(event.locations as unknown[] | undefined) ??
            existing.locations;
          const newRawInput =
            event.rawInput !== undefined && event.rawInput !== null ? event.rawInput : existing.rawInput;
          const newRawOutput =
            event.rawOutput !== undefined && event.rawOutput !== null ? event.rawOutput : existing.rawOutput;
          const newTerminalMeta =
            event.terminalMeta !== undefined ? event.terminalMeta : existing.terminalMeta;
          const updated: AppToolCall = {
            ...existing,
            roleName: existing.roleName ?? roleName,
            toolName: event.toolName ?? existing.toolName,
            parentId: event.parentId ?? existing.parentId ?? null,
            diff: event.diff !== undefined ? event.diff : existing.diff,
            kind: event.toolKind ?? existing.kind,
            status: normalizeToolStatus(event.status) ?? existing.status,
            title: event.title ?? existing.title,
            content: newContent,
            locations: newLocations,
            rawInput: newRawInput,
            rawOutput: newRawOutput,
            terminalMeta: newTerminalMeta,
          };
          s.toolCalls[event.toolCallId!] = updated;
          if (isNew) {
            s.streamSegments.push({ kind: "tool", tc: updated, roleName });
          } else {
            for (let i = s.streamSegments.length - 1; i >= 0; i--) {
              const seg = s.streamSegments[i];
              if (seg.kind === "tool" && seg.tc.toolCallId === event.toolCallId) {
                s.streamSegments[i] = { kind: "tool", tc: updated, roleName: seg.roleName ?? roleName };
                break;
              }
            }
          }
          applyTerminalMetaToSession(s, event, updated);
        });
        if (event.status || event.title) {
          patchSession(sid, {
            agentState: `${event.toolName ?? event.toolKind ?? "tool"} ${event.status ?? "updated"}: ${
              event.title ?? event.toolCallId
            }`,
          });
        }
        scheduleScrollToBottom();
      }
      break;
    case "toolOutputDelta":
      // Appended rather than replacing rawOutput so a long command's stdout can render as
      // it arrives without the provider resending the whole call each chunk. Chunks are
      // coalesced per frame: one store write per row per frame instead of one per chunk.
      if (event.toolCallId && event.delta) queueToolOutput(sid, event.toolCallId, event.delta, deps.mutateSession);
      break;
    case "userInputRequest":
      if (event.requestId) {
        mutateSession(sid, (s) => {
          const next = {
            requestId: event.requestId!,
            title: event.title ?? null,
            blocking: event.blocking ?? true,
            questions: event.questions ?? [],
          };
          const rest = (s.pendingUserInput ?? []).filter((q) => q.requestId !== next.requestId);
          s.pendingUserInput = [...rest, next];
        });
      }
      break;
    case "contextCompacted":
      // Surfaced as a notice: compaction silently changes what the agent can still recall,
      // so it should not pass unannounced.
      mutateSession(sid, (s) => {
        const before = event.beforeTokens;
        const after = event.afterTokens;
        const detail =
          before && after ? ` (${before} → ${after} tokens)` : before ? ` (was ${before} tokens)` : "";
        s.notices = [
          ...(s.notices ?? []).slice(-19),
          {
            id: `compact-${Date.now()}`,
            level: "info",
            code: "context_compacted",
            text: `Conversation compacted${detail}`,
            at: Date.now(),
          },
        ];
      });
      break;
    case "usage":
      // Providers report cumulative or per-message counters depending on the CLI; merging
      // rather than replacing keeps fields a later frame omits.
      mutateSession(sid, (s) => {
        const prev = s.usage;
        s.usage = {
          inputTokens: event.inputTokens ?? prev?.inputTokens ?? null,
          outputTokens: event.outputTokens ?? prev?.outputTokens ?? null,
          cacheReadTokens: event.cacheReadTokens ?? prev?.cacheReadTokens ?? null,
          cacheWriteTokens: event.cacheWriteTokens ?? prev?.cacheWriteTokens ?? null,
          reasoningTokens: event.reasoningTokens ?? prev?.reasoningTokens ?? null,
          totalTokens: event.totalTokens ?? prev?.totalTokens ?? null,
          contextWindow: event.contextWindow ?? prev?.contextWindow ?? null,
          costUsd: event.costUsd ?? prev?.costUsd ?? null,
        };
      });
      break;
    case "notice":
      if (event.text) {
        mutateSession(sid, (s) => {
          s.notices = [
            ...(s.notices ?? []).slice(-19),
            {
              id: `${Date.now()}-${(s.notices ?? []).length}`,
              level: event.level ?? "info",
              code: event.code ?? null,
              text: event.text!,
              at: Date.now(),
            },
          ];
        });
      }
      break;
    case "plan":
      // An empty plan is a removed one (ACP `plan_removed`, or a plan cleared to no steps).
      if (event.entries) {
        patchSession(sid, { currentPlan: event.entries.length > 0 ? (event.entries as AppPlanEntry[]) : null });
      }
      break;
    case "permissionRequest":
      if (event.requestId) {
        mutateSession(sid, (s) => {
          const next: AppPermission = {
            requestId: event.requestId!,
            toolCallId: event.parentId ?? event.toolCallId ?? null,
            title: event.title ?? "Permission Required",
            description: event.description ?? null,
            options: normalizePermissionOptions(event.options),
            toolKind: event.toolKind ?? null,
            content: (event.content as unknown[] | undefined) ?? null,
            rawInput: event.rawInput,
          };
          const existing = s.pendingPermissions?.findIndex((p) => p.requestId === next.requestId);
          if (existing !== undefined && existing >= 0) {
            s.pendingPermissions = s.pendingPermissions.map((p) =>
              p.requestId === next.requestId ? next : p,
            );
          } else {
            s.pendingPermissions = [...(s.pendingPermissions ?? []), next];
          }
        });
      }
      break;
    case "permissionExpired":
      if (event.requestId) {
        mutateSession(sid, (s) => {
          s.pendingPermissions = (s.pendingPermissions ?? []).filter(
            (p) => p.requestId !== event.requestId,
          );
          s.pendingUserInput = (s.pendingUserInput ?? []).filter(
            (q) => q.requestId !== event.requestId,
          );
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
    case "unknown":
      mutateSession(sid, (s) => {
        s.streamSegments.push({
          kind: "other",
          type: event.typeName ?? "unknown",
          payload: event.raw ?? event,
          roleName,
        });
      });
      scheduleScrollToBottom();
      break;
    default:
      mutateSession(sid, (s) => {
        s.streamSegments.push({
          kind: "other",
          type: event.kind || "unknown",
          payload: event,
          roleName,
        });
      });
      scheduleScrollToBottom();
      break;
  }
}
