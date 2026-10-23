import type { AppMessage, AppSession } from "../components/types";

export function responseMessage(message: AppMessage): AppMessage | null {
  if (message.roleName === "user") return message;
  if (message.payload?.kind === "turnError" || message.payload?.kind === "turnCancelled") return message;
  if (["system", "event"].includes(message.roleName)) return null;
  if (message.messageType && ["tool", "thought", "event", "context"].includes(message.messageType)) return null;
  if (message.purpose && ["providerEvent", "roleHandoff", "taskState"].includes(message.purpose)) return null;
  // Settled turns carry the authoritative reply in text; segments also contain commentary.
  const text = message.text || message.segments?.filter((s) => s.kind === "text").map((s) => s.kind === "text" ? s.text : "").join("\n\n") || "";
  if (!text.trim() && !message.images?.length) return null;
  return { ...message, text, segments: undefined, toolCalls: undefined, thoughtText: undefined };
}

export function responseSession(session: AppSession | null): AppSession | null {
  if (!session) return null;
  const liveId = session.submitting ? session.streamingMessage?.id : undefined;
  return {
    ...session,
    messages: session.messages.filter((m) => m.id !== liveId).map(responseMessage).filter((m): m is AppMessage => m !== null),
    streamingMessage: null,
    streamSegments: [],
    toolCalls: {},
    thoughtText: "",
    agentState: session.submitting ? "Agent is working…" : undefined,
    currentPlan: null,
    currentMode: null,
    usage: null,
    notices: [],
  };
}
