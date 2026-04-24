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
} from "./acpEventBridge";

type PrewarmStage = "warming" | "ready" | "failed";
type PrewarmRuntimeState = { stage: PrewarmStage; error?: string };
type PrewarmScopeState = {
  messageId: string;
  runtimes: Map<string, PrewarmRuntimeState>;
};

export type AcpConnectionLostPayload = {
  runtimeKey: string;
  roleName: string;
  appSessionId: string;
  reason?: string | null;
};

export type AcpPrewarmPayload = {
  runtimeKey: string;
  roleName: string;
  appSessionId: string;
  status: string | { failed: { error: string } };
};

export type AcpStreamPayload = {
  role: string;
  runtimeKind?: string;
  appSessionId?: string;
  seq?: number;
  event: AcpStreamEvent;
};

export type AcpEventBusDeps = {
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

export function createAcpEventBus(deps: AcpEventBusDeps) {
  const lastSeqBySession = new Map<string, number>();
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
      deps.pushMessage("event", text);
      return;
    }

    deps.mutateSession(appSessionId, (s) => {
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
    deps.scheduleScrollToBottom();
  };

  return {
    jockeyMcpError(payload: string) {
      deps.pushMessage("event", `Jockey MCP bridge failed: ${payload}`);
    },
    connectionLost(payload: AcpConnectionLostPayload) {
      const sid = payload.appSessionId;
      const msg = toConnectionLostMessage(payload);
      if (sid) {
        deps.mutateSession(sid, (s) => {
          s.pendingPermission = null;
          s.agentState = payload.reason ?? "Disconnected";
        });
        deps.pushMessageToSession(sid, "event", msg);
      } else {
        deps.pushMessage("event", msg);
      }
    },
    prewarm(payload: AcpPrewarmPayload) {
      const { runtimeKey, appSessionId, status } = payload;
      if (status === "started") {
        upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "warming" });
      } else if (status === "ready") {
        upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "ready" });
      } else if (typeof status === "object" && status && "failed" in status) {
        upsertPrewarmMessage(appSessionId, runtimeKey, { stage: "failed", error: status.failed.error });
      }
    },
    delta(payload: AcpDeltaEvent & { appSessionId?: string }) {
      appendAcpDelta(payload, deps.acceptingStreams, deps.sessions, deps.appendStream, deps.getSessionIndex);
    },
    sessionUpdate(payload: SessionUpdateEvent & { appSessionId?: string }) {
      const line = toSessionDeltaMessage(payload);
      if (line) deps.onSessionDeltaLine(payload.appSessionId ?? "", line);
    },
    workflowState(payload: WorkflowStateEvent & { appSessionId?: string }) {
      const msg = toWorkflowStateMessage(payload);
      const sid = payload.appSessionId;
      if (sid) {
        deps.pushMessageToSession(sid, "event", msg);
      } else {
        deps.pushMessage("event", msg);
      }
    },
    stream(payload: AcpStreamPayload) {
      const sid = payload.appSessionId;
      if (!sid) return;
      if (!deps.acceptingStreams.has(sid)) {
        console.debug("[acp/stream] dropped (not accepting)", { sid, seq: payload.seq, kind: payload.event?.kind });
        return;
      }
      const seq = payload.seq;
      if (typeof seq === "number") {
        const prev = lastSeqBySession.get(sid);
        if (prev !== undefined && seq !== prev + 1) {
          console.warn("[acp/stream] seq gap", { sid, prev, seq, gap: seq - prev - 1, kind: payload.event?.kind });
        }
        lastSeqBySession.set(sid, seq);
      }
      applyAcpStreamEvent({
        sid,
        roleName: payload.role,
        runtimeKind: payload.runtimeKind,
        event: payload.event,
        patchSession: (sessionId, patch) => deps.updateSession(sessionId, patch),
        mutateSession: deps.mutateSession,
        appendThought: deps.appendThought,
        normalizeToolLocations: deps.normalizeToolLocations,
        parseAgentCommands: deps.parseAgentCommands,
        normalizeRuntimeKey: deps.normalizeRuntimeKey,
        roles: deps.roles,
        commandCacheKey: deps.commandCacheKey,
        scheduleScrollToBottom: deps.scheduleScrollToBottom,
      });
    },
    clear() {
      prewarmState.clear();
      lastSeqBySession.clear();
    },
  };
}
