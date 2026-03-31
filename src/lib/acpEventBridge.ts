import { produce } from "solid-js/store";
import type {
  AcpDeltaEvent,
  AcpStreamEvent,
  AppPlanEntry,
  AppSession,
  AppToolCall,
  Role,
  SessionUpdateEvent,
  WorkflowStateEvent,
} from "../components/types";

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
}): string {
  return `Agent ${payload.runtimeKey} (role: ${payload.roleName}) disconnected — will reconnect on next message.`;
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
  acceptingStreams: Set<string>,
  sessions: AppSession[],
  appendStream: (sid: string, chunk: string) => void,
): void {
  const sid = payload.appSessionId;
  if (!sid || !acceptingStreams.has(sid)) return;
  const sess = sessions.find((s) => s.id === sid);
  if (!sess?.streamingMessage) return;
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
    default:
      break;
  }
}

export function mutateSessionWithProduce(
  sid: string,
  setSessions: (
    selector: (s: AppSession) => boolean,
    recipe: (s: AppSession) => void,
  ) => void,
  recipe: (s: AppSession) => void,
) {
  setSessions((s) => s.id === sid, produce(recipe));
}
