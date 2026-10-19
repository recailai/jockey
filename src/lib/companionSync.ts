import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppMessage, AppSession, TurnFinishedEvent, TurnStartedEvent } from "../components/types";
import { makeDefaultSession } from "./sessionHelpers";
import { appSessionApi, type RawSession } from "./tauriApi";

type CompanionChange = {
  kind: "projects" | "sessions";
  sessionId?: string | null;
};

type CompanionSyncInput = {
  sessions: AppSession[];
  appendSessions: (fresh: AppSession[]) => void;
  adoptMessages: (sessionId: string, messages: readonly AppMessage[]) => void;
  acceptingStreams: Map<string, number>;
  refreshProjects: () => Promise<void>;
};

const sessionFromRaw = (r: RawSession): AppSession => {
  const s = makeDefaultSession(r.title);
  s.id = r.id;
  if (r.activeRole) s.activeRole = r.activeRole;
  if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
  if (r.runtimeProfileId !== undefined) s.runtimeProfileId = r.runtimeProfileId;
  if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
  s.projectId = r.projectId ?? null;
  s.messages = r.messages ?? [];
  return s;
};

/**
 * Keeps the desktop store in step with turns and sessions it did not start itself (the web
 * companion, the Jockey MCP bridge). Turns are recorded by the engine; this only mirrors the
 * engine's `turn/started` / `turn/finished` announcements. Turns this window started are
 * skipped — `sendRaw` adopts their messages from the command result.
 */
export async function registerCompanionSync(input: CompanionSyncInput): Promise<UnlistenFn> {
  const ownTurn = (sid: string) => input.acceptingStreams.has(sid);
  const known = (sid: string) => input.sessions.some((s) => s.id === sid);

  const unlisteners = await Promise.all([
    listen<CompanionChange>("companion/changed", async (ev) => {
      if (ev.payload.kind === "projects") {
        await input.refreshProjects();
        return;
      }
      let raw: RawSession[];
      try {
        raw = await appSessionApi.list();
      } catch {
        return;
      }
      const fresh = raw.filter((r) => !known(r.id)).map(sessionFromRaw);
      if (fresh.length > 0) input.appendSessions(fresh);
    }),
    listen<TurnStartedEvent>("turn/started", (ev) => {
      const { appSessionId, userMessage } = ev.payload;
      if (!known(appSessionId) || ownTurn(appSessionId)) return;
      // The announcement carries an image count, not the image data.
      const { images, ...rest } = userMessage as AppMessage & { images?: unknown };
      input.adoptMessages(appSessionId, [Array.isArray(images) ? userMessage : rest]);
    }),
    listen<TurnFinishedEvent>("turn/finished", (ev) => {
      const { appSessionId, messages } = ev.payload;
      if (!known(appSessionId) || ownTurn(appSessionId)) return;
      input.adoptMessages(appSessionId, messages);
    }),
  ]);
  return () => unlisteners.forEach((u) => u());
}
