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

/** What the agent is doing right now, for the one-line progress under a live reply. */
function liveActivity(session: AppSession): string {
  const running = Object.values(session.toolCalls ?? {}).filter((tc) => tc.status === "running" || tc.status === "pending");
  const current = running[running.length - 1];
  if (current?.title) return `${current.title}…`;
  const done = Object.keys(session.toolCalls ?? {}).length;
  return done ? `Agent is working… (${done} step${done === 1 ? "" : "s"} so far)` : "Agent is working…";
}

/**
 * The phone's reading view: replies without tool cards. The live turn keeps its streamed text
 * (it used to be hidden until the turn ended, so a long turn looked like no output at all and
 * the previous reply read as the latest one) plus a one-line activity indicator.
 */
export function responseSession(session: AppSession | null): AppSession | null {
  if (!session) return null;
  const live = session.submitting ? session.streamingMessage : null;
  return {
    ...session,
    // The live turn's checkpoint row and its stream are the same reply; keep only the stream.
    messages: session.messages.filter((m) => m.id !== live?.id).map(responseMessage).filter((m): m is AppMessage => m !== null),
    streamingMessage: live ? { ...live, text: live.text ?? "" } : null,
    streamSegments: [],
    toolCalls: {},
    thoughtText: "",
    agentState: session.submitting ? liveActivity(session) : undefined,
    currentPlan: null,
    currentMode: null,
    usage: null,
    notices: [],
  };
}
