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
import { now } from "../components/types";
import {
  appendAcpDelta,
  applyAcpStreamEvent,
  toConnectionLostMessage,
  toSessionDeltaMessage,
  toWorkflowStateMessage,
} from "../lib/acpEventBridge";

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

    type PrewarmStage = "warming" | "ready" | "failed";
    type PrewarmRuntimeState = { stage: PrewarmStage; error?: string };
    type PrewarmScopeState = {
      messageId: string;
      runtimes: Map<string, PrewarmRuntimeState>;
    };
    const prewarmState = new Map<string, PrewarmScopeState>();

    const renderPrewarmText = (runtimes: Map<string, PrewarmRuntimeState>): string => {
      const warming: string[] = [];
      const ready: string[] = [];
      const failed: string[] = [];
      for (const [runtimeKey, state] of runtimes) {
        if (state.stage === "warming") warming.push(runtimeKey);
        if (state.stage === "ready") ready.push(runtimeKey);
        if (state.stage === "failed") failed.push(`${runtimeKey}: ${state.error ?? "unknown error"}`);
      }
      const parts: string[] = [];
      if (warming.length > 0) parts.push(`warming ${warming.join(", ")}`);
      if (ready.length > 0) parts.push(`ready ${ready.join(", ")}`);
      if (failed.length > 0) parts.push(`failed ${failed.join("; ")}`);
      return parts.length > 0 ? `Runtime warmup: ${parts.join(" | ")}` : "Runtime warmup: idle";
    };

    const upsertPrewarmMessage = (appSessionId: string, runtimeKey: string, state: PrewarmRuntimeState) => {
      const scopeKey = appSessionId || "__global__";
      const scope = prewarmState.get(scopeKey) ?? {
        messageId: `runtime-warmup-${scopeKey}`,
        runtimes: new Map<string, PrewarmRuntimeState>(),
      };
      scope.runtimes.set(runtimeKey, state);
      prewarmState.set(scopeKey, scope);
      const text = renderPrewarmText(scope.runtimes);

      if (!appSessionId) {
        pushMessage("event", text);
        return;
      }

      mutateSession(appSessionId, (s) => {
        const idx = s.messages.findIndex((m) => m.id === scope.messageId);
        if (idx === -1) {
          s.messages.push({
            id: scope.messageId,
            roleName: "event",
            text,
            at: now(),
          });
          return;
        }
        s.messages[idx].text = text;
        s.messages[idx].at = now();
      });
      scheduleScrollToBottom();
    };

    const listeners = await Promise.all([
      listen<string>("jockey-mcp/error", (ev) => {
        pushMessage("event", `Jockey MCP bridge failed: ${ev.payload}`);
      }),
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
      listen<{ runtimeKey: string; roleName: string; appSessionId: string; status: string | { failed: { error: string } } }>(
        "acp/prewarm",
        (ev) => {
          const { runtimeKey, appSessionId, status } = ev.payload;
          if (status === "started") {
            upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "warming" });
          } else if (status === "ready") {
            upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "ready" });
          } else if (typeof status === "object" && status && "failed" in status) {
            const err = (status as { failed: { error: string } }).failed.error;
            upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "failed", error: err });
          }
        },
      ),
      listen<AcpDeltaEvent & { appSessionId?: string }>("acp/delta", (ev) => {
        appendAcpDelta(ev.payload, acceptingStreams, sessions, appendStream, getSessionIndex);
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

    listeners.push(() => {
      prewarmState.clear();
    });

    return listeners;
  };

  return { registerAcpEventListeners };
}
