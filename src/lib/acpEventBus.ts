import type { Accessor } from "solid-js";
import type {
  AcpDeltaEvent,
  AcpStreamPayload,
  AppSession,
  Role,
  SessionUpdateEvent,
  WorkflowStateEvent,
} from "../components/types";
import { now } from "../components/types";
import {
  appendAcpDelta,
  toConnectionLostMessage,
  toSessionDeltaMessage,
  toWorkflowStateMessage,
} from "./acpEventBridge";
import { acceptsAgentEvent, applyAgentEvent, normalizeAgentEvent } from "./agentEvent";

export type { AcpStreamPayload } from "../components/types";

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

export type AcpEventBusDeps = {
  acceptingStreams: Map<string, number>;
  sessions: AppSession[];
  getSessionIndex: (id: string) => number;
  appendStream: (sid: string, chunk: string, roleName?: string) => void;
  pushMessageToSession: (sid: string, role: string, text: string) => void;
  pushMessage: (role: string, text: string) => void;
  onSessionDeltaLine: (sid: string, line: string, roleName?: string) => void;
  updateSession: (id: string, patch: Partial<AppSession>) => void;
  mutateSession: (sid: string, recipe: (s: AppSession) => void) => void;
  appendThought: (sid: string, text: string, roleName?: string) => void;
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
  scheduleCheckpoint?: (sid: string) => void;
};

export function createAcpEventBus(deps: AcpEventBusDeps) {
  const lastSeqByStream = new Map<string, number>();
  const activeTurnBySession = new Map<string, { runToken?: number; turnId: string }>();
  const retiredTurnsBySession = new Map<string, Set<string>>();
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
          s.pendingPermissions = [];
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
      const sid = payload.appSessionId?.trim();
      if (sid && payload.turnId) {
        if (retiredTurnsBySession.get(sid)?.has(payload.turnId)) {
          console.debug("[agent/event] dropped (retired delta turn)", {
            sid,
            turnId: payload.turnId,
          });
          return;
        }
        const currentTurn = activeTurnBySession.get(sid);
        const currentToken = deps.acceptingStreams.get(sid);
        if (currentTurn && currentTurn.runToken === currentToken && currentTurn.turnId !== payload.turnId) {
          console.debug("[agent/event] dropped (stale delta turn)", {
            sid,
            expected: currentTurn.turnId,
            received: payload.turnId,
          });
          return;
        }
        activeTurnBySession.set(sid, { runToken: currentToken, turnId: payload.turnId });
      }
      appendAcpDelta(payload, deps.acceptingStreams, deps.sessions, deps.appendStream, deps.getSessionIndex);
    },
    sessionUpdate(payload: SessionUpdateEvent & { appSessionId?: string }) {
      const line = toSessionDeltaMessage(payload);
      const sid = payload.appSessionId?.trim() || payload.sessionId?.trim() || "";
      if (line) deps.onSessionDeltaLine(sid, line.text, line.roleName);
    },
    workflowState(payload: WorkflowStateEvent & { appSessionId?: string }) {
      const msg = toWorkflowStateMessage(payload);
      const sid = payload.appSessionId?.trim() || payload.sessionId?.trim() || "";
      if (sid) {
        deps.pushMessageToSession(sid, "event", msg);
      } else {
        deps.pushMessage("event", msg);
      }
    },
    stream(payload: AcpStreamPayload) {
      const sid = payload.appSessionId?.trim();
      if (!sid) return;
      if (!deps.acceptingStreams.has(sid)) {
        console.debug("[agent/event] dropped (not accepting)", { sid, seq: payload.seq, kind: payload.event?.kind });
        return;
      }
      const currentToken = deps.acceptingStreams.get(sid);
      const idx = deps.getSessionIndex(sid);
      const sessionRunToken = idx !== -1 ? deps.sessions[idx]?.streamingRunToken : undefined;
      if (currentToken !== undefined && sessionRunToken !== currentToken) {
        console.debug("[agent/event] dropped (stale run token)", { sid, seq: payload.seq, kind: payload.event?.kind });
        return;
      }
      const envelope = normalizeAgentEvent(payload, sessionRunToken !== undefined ? String(sessionRunToken) : undefined);
      if (!envelope) return;
      if (retiredTurnsBySession.get(sid)?.has(envelope.turnId)) {
        console.debug("[agent/event] dropped (retired stream turn)", {
          sid,
          turnId: envelope.turnId,
          kind: envelope.event.kind,
        });
        return;
      }
      const currentTurn = activeTurnBySession.get(sid);
      const isControlPlaneEvent = ["permissionRequest", "permissionExpired", "userInputRequest"].includes(
        envelope.event.kind,
      );
      if (!isControlPlaneEvent && !envelope.turnId.startsWith("legacy:")) {
        if (currentTurn && currentTurn.runToken === currentToken && currentTurn.turnId !== envelope.turnId) {
          console.debug("[agent/event] dropped (stale turn)", {
            sid,
            expected: currentTurn.turnId,
            received: envelope.turnId,
            kind: envelope.event.kind,
          });
          return;
        }
        activeTurnBySession.set(sid, { runToken: currentToken, turnId: envelope.turnId });
      }
      const session = idx !== -1 ? deps.sessions[idx] : undefined;
      if (!acceptsAgentEvent(session, envelope)) return;
      const seq = envelope.seq;
      const streamKey = `${sid}:${envelope.turnId}`;
      if (typeof seq === "number" && seq > 0) {
        const prev = lastSeqByStream.get(streamKey);
        if (prev !== undefined && seq <= prev) {
          console.debug("[agent/event] dropped (duplicate/out of order)", {
            sid,
            turnId: envelope.turnId,
            prev,
            seq,
            kind: envelope.event?.kind,
          });
          return;
        }
        if (prev !== undefined && seq !== prev + 1) {
          console.warn("[agent/event] seq gap", { sid, prev, seq, gap: seq - prev - 1, kind: envelope.event?.kind });
        }
        lastSeqByStream.set(streamKey, seq);
      }
      applyAgentEvent({
        sid,
        patchSession: (sessionId, patch) => deps.updateSession(sessionId, patch),
        mutateSession: deps.mutateSession,
        appendThought: deps.appendThought,
        normalizeToolLocations: deps.normalizeToolLocations,
        parseAgentCommands: deps.parseAgentCommands,
        normalizeRuntimeKey: deps.normalizeRuntimeKey,
        roles: deps.roles,
        commandCacheKey: deps.commandCacheKey,
        scheduleScrollToBottom: deps.scheduleScrollToBottom,
        scheduleCheckpoint: deps.scheduleCheckpoint,
      }, envelope);
    },
    clearSession(sid: string) {
      const activeTurn = activeTurnBySession.get(sid);
      if (activeTurn && !activeTurn.turnId.startsWith("legacy:")) {
        const retired = retiredTurnsBySession.get(sid) ?? new Set<string>();
        retired.add(activeTurn.turnId);
        while (retired.size > 8) retired.delete(retired.values().next().value as string);
        retiredTurnsBySession.set(sid, retired);
      }
      for (const key of lastSeqByStream.keys()) {
        if (key.startsWith(`${sid}:`)) lastSeqByStream.delete(key);
      }
      activeTurnBySession.delete(sid);
      prewarmState.delete(sid);
    },
    clear() {
      prewarmState.clear();
      lastSeqByStream.clear();
      activeTurnBySession.clear();
      retiredTurnsBySession.clear();
    },
  };
}
