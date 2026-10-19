import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSession } from "../components/types";
import { makeDefaultSession } from "./sessionHelpers";
import { appSessionApi, type RawSession } from "./tauriApi";

type CompanionChange = {
  kind: "projects" | "sessions" | "messages";
  sessionId?: string | null;
};

type CompanionSyncInput = {
  sessions: AppSession[];
  appendSessions: (fresh: AppSession[]) => void;
  getSessionIndex: (id: string) => number;
  mutateSession: (id: string, recipe: (s: AppSession) => void) => void;
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
 * Keeps the desktop store in step with the remote web companion. The companion writes
 * projects, sessions and whole turns straight to SQLite, and the desktop never renders
 * those turns' streams (they are not in `acceptingStreams`), so the rows are pulled back in
 * whenever the gateway reports a change.
 */
export async function registerCompanionSync(input: CompanionSyncInput): Promise<UnlistenFn> {
  return listen<CompanionChange>("companion/changed", async (ev) => {
    const { kind, sessionId } = ev.payload;
    if (kind === "projects") {
      await input.refreshProjects();
      return;
    }
    let raw: RawSession[];
    try {
      raw = await appSessionApi.list();
    } catch {
      return;
    }
    const known = new Set(input.sessions.map((s) => s.id));
    const fresh = raw.filter((r) => !known.has(r.id)).map(sessionFromRaw);
    if (fresh.length > 0) input.appendSessions(fresh);

    if (kind !== "messages" || !sessionId) return;
    const row = raw.find((r) => r.id === sessionId);
    const idx = input.getSessionIndex(sessionId);
    // A desktop-initiated turn owns this session's messages until it finalizes; replacing
    // them mid-stream would drop the in-flight checkpoint.
    if (!row || idx === -1 || input.acceptingStreams.has(sessionId)) return;
    if (input.sessions[idx].submitting) return;
    input.mutateSession(sessionId, (s) => {
      s.messages = row.messages ?? [];
    });
  });
}
