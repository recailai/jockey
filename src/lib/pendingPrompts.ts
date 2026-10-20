import type { AppSession } from "../components/types";
import { assistantApi } from "./tauriApi";

type Deps = {
  getSessionIndex: (id: string) => number;
  mutateSession: (sid: string, recipe: (s: AppSession) => void) => void;
};

/**
 * Stream events are not replayed, so a view that (re)loads pulls the prompts the agent is
 * blocked on right now. With `sessionIds`, prompts of those sessions that are no longer
 * pending are dropped too: another client answered them while this one was not looking.
 */
export async function hydratePendingPrompts(
  { getSessionIndex, mutateSession }: Deps,
  sessionIds?: readonly string[],
) {
  const pending = await assistantApi.listPendingPrompts();
  const permissionIds = new Set(pending.permissions.map((p) => p.requestId));
  const inputIds = new Set(pending.userInputs.map((u) => u.requestId));
  for (const sid of sessionIds ?? []) {
    mutateSession(sid, (s) => {
      s.pendingPermissions = (s.pendingPermissions ?? []).filter((p) => permissionIds.has(p.requestId));
      s.pendingUserInput = (s.pendingUserInput ?? []).filter((u) => inputIds.has(u.requestId));
    });
  }
  for (const p of pending.permissions) {
    if (getSessionIndex(p.appSessionId) === -1) continue;
    mutateSession(p.appSessionId, (s) => {
      if (!(s.pendingPermissions ?? []).some((x) => x.requestId === p.requestId)) {
        s.pendingPermissions = [...(s.pendingPermissions ?? []), p];
      }
    });
  }
  for (const u of pending.userInputs) {
    if (getSessionIndex(u.appSessionId) === -1) continue;
    mutateSession(u.appSessionId, (s) => {
      if (!(s.pendingUserInput ?? []).some((x) => x.requestId === u.requestId)) {
        s.pendingUserInput = [...(s.pendingUserInput ?? []), u];
      }
    });
  }
}
