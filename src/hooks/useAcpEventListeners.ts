import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Accessor } from "solid-js";
import type {
  AcpDeltaEvent,
  AcpStreamEvent,
  AppSession,
  Role,
  SessionUpdateEvent,
  WorkflowStateEvent,
} from "../components/types";
import {
  appendAcpDelta,
  applyAcpStreamEvent,
  toConnectionLostMessage,
  toSessionDeltaMessage,
  toWorkflowStateMessage,
} from "../lib/acpEventBridge";

type RegisterAcpEventListenersInput = {
  acceptingStreams: Set<string>;
  sessions: AppSession[];
  appendStream: (sid: string, chunk: string) => void;
  pushMessageToSession: (sid: string, role: string, text: string) => void;
  pushMessage: (role: string, text: string) => void;
  onSessionDeltaLine: (sid: string, line: string) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  mutateSession: (sid: string, recipe: (s: AppSession) => void) => void;
  appendThought: (sid: string, text: string) => void;
  normalizeToolLocations: (
    raw: unknown[] | undefined,
  ) => Array<{ path: string; line?: number }> | undefined;
  parseAgentCommands: (
    raw: unknown[],
  ) => Array<{ name: string; description: string; hint?: string }>;
  normalizeRuntimeKey: (runtimeKey: string) => string;
  roles: Accessor<Role[]>;
  commandCacheKey: (runtimeKey: string, roleName: string) => string;
  scheduleScrollToBottom: () => void;
};

export function useAcpEventListeners() {
  const registerAcpEventListeners = async (
    input: RegisterAcpEventListenersInput,
  ): Promise<UnlistenFn[]> => {
    const {
      acceptingStreams,
      sessions,
      appendStream,
      pushMessageToSession,
      pushMessage,
      onSessionDeltaLine,
      updateSession,
      mutateSession,
      appendThought,
      normalizeToolLocations,
      parseAgentCommands,
      normalizeRuntimeKey,
      roles,
      commandCacheKey,
      scheduleScrollToBottom,
    } = input;

    return Promise.all([
      listen<{ runtimeKey: string; roleName: string; appSessionId: string }>(
        "acp/connection-lost",
        (ev) => {
          const sid = ev.payload.appSessionId;
          const msg = toConnectionLostMessage(ev.payload);
          if (sid) {
            pushMessageToSession(sid, "event", msg);
          } else {
            pushMessage("event", msg);
          }
        },
      ),
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        appendAcpDelta(ev.payload, acceptingStreams, sessions, appendStream);
      }),
      listen<SessionUpdateEvent & { appSessionId?: string }>("session/update", (ev) => {
        const line = toSessionDeltaMessage(ev.payload);
        if (!line) return;
        onSessionDeltaLine(ev.payload.appSessionId ?? "", line);
      }),
      listen<WorkflowStateEvent & { appSessionId?: string }>("workflow/state_changed", (ev) => {
        const msg = toWorkflowStateMessage(ev.payload);
        const sid = ev.payload.appSessionId;
        if (sid) {
          pushMessageToSession(sid, "event", msg);
        } else {
          pushMessage("event", msg);
        }
      }),
      listen<{ role: string; runtimeKind?: string; appSessionId?: string; event: AcpStreamEvent }>(
        "acp/stream",
        (ev) => {
          const sid = ev.payload.appSessionId;
          if (!sid || !acceptingStreams.has(sid)) return;
          applyAcpStreamEvent({
            sid,
            roleName: ev.payload.role,
            runtimeKind: ev.payload.runtimeKind,
            event: ev.payload.event,
            patchSession: (sessionId, patch) => updateSession(sessionId, patch),
            mutateSession,
            appendThought,
            normalizeToolLocations,
            parseAgentCommands,
            normalizeRuntimeKey,
            roles,
            commandCacheKey,
            scheduleScrollToBottom,
          });
        },
      ),
    ]);
  };

  return { registerAcpEventListeners };
}
