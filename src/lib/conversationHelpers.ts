import type { AppSession } from "../components/types";

/** True when the session has no user/agent conversation content yet. */
export function hasConversationContent(session: AppSession | null | undefined): boolean {
  if (!session) return false;
  if (session.submitting || session.streamingMessage) return true;
  if ((session.queuedMessages?.length ?? 0) > 0) return true;
  return session.messages.some((m) => {
    if (m.roleName === "user") return true;
    if (m.roleName === "system" || m.roleName === "event") return false;
    return !!m.text?.trim();
  });
}

export const CONVERSATION_HERO_TITLE = "What should we work on?";
export const CONVERSATION_HERO_SUBTITLE = "Start a conversation or open files and diffs from the side panels.";
