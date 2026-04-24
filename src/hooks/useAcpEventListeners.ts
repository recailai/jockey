import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Accessor } from "solid-js";
import type {
  AcpDeltaEvent,
  AppSession,
  Role,
  SessionUpdateEvent,
  WorkflowStateEvent,
} from "../components/types";
import {
  createAcpEventBus,
  type AcpConnectionLostPayload,
  type AcpPrewarmPayload,
  type AcpStreamPayload,
} from "../lib/acpEventBus";

type RegisterAcpEventListenersInput = {
  acceptingStreams: Map<string, number>;
  sessions: AppSession[];
  getSessionIndex: (id: string) => number;
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
      getSessionIndex,
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

    const bus = createAcpEventBus({
      acceptingStreams,
      sessions,
      getSessionIndex,
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
    });

    const listeners = await Promise.all([
      listen<string>("jockey-mcp/error", (ev) => {
        bus.jockeyMcpError(ev.payload);
      }),
      listen<AcpConnectionLostPayload>(
        "acp/connection-lost",
        (ev) => {
          bus.connectionLost(ev.payload);
        },
      ),
      listen<AcpPrewarmPayload>(
        "acp/prewarm",
        (ev) => {
          bus.prewarm(ev.payload);
        },
      ),
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        bus.delta(ev.payload);
      }),
      listen<SessionUpdateEvent & { appSessionId?: string }>("session/update", (ev) => {
        bus.sessionUpdate(ev.payload);
      }),
      listen<WorkflowStateEvent & { appSessionId?: string }>("workflow/state_changed", (ev) => {
        bus.workflowState(ev.payload);
      }),
      listen<AcpStreamPayload>(
        "acp/stream",
        (ev) => {
          bus.stream(ev.payload);
        },
      ),
    ]);

    listeners.push(() => {
      bus.clear();
    });

    return listeners;
  };

  return { registerAcpEventListeners };
}
