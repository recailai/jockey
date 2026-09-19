import type { AgentEventEnvelope, AcpStreamPayload, AppSession } from "../components/types";
import { applyAcpStreamEvent, type BridgeDeps } from "./acpEventBridge";

export const AGENT_EVENT_SCHEMA_VERSION = 1;

export function normalizeAgentEvent(payload: AcpStreamPayload, fallbackTurnId?: string): AgentEventEnvelope | null {
  const sessionId = payload.appSessionId?.trim();
  if (!sessionId) return null;
  return {
    schemaVersion: payload.schemaVersion ?? AGENT_EVENT_SCHEMA_VERSION,
    sessionId,
    turnId: payload.turnId ?? (fallbackTurnId ? `run:${fallbackTurnId}` : `legacy:${sessionId}`),
    roleName: payload.role,
    runtimeKey: payload.runtimeKind ?? "unknown",
    seq: payload.seq ?? 0,
    event: payload.event,
  };
}

export function acceptsAgentEvent(
  session: AppSession | undefined,
  envelope: AgentEventEnvelope,
  activeTurnId?: string,
): boolean {
  if (!session || session.id !== envelope.sessionId) return false;
  if (activeTurnId && activeTurnId !== envelope.turnId && envelope.turnId !== `legacy:${envelope.sessionId}`) return false;
  return true;
}

export function applyAgentEvent(deps: Omit<BridgeDeps, "event" | "sid" | "roleName" | "runtimeKind"> & { sid: string }, envelope: AgentEventEnvelope): void {
  applyAcpStreamEvent({
    ...deps,
    sid: envelope.sessionId,
    roleName: envelope.roleName,
    runtimeKind: envelope.runtimeKey,
    event: envelope.event,
  });
}
