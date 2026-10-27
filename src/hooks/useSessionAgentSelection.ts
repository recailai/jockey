import { createEffect, onCleanup } from "solid-js";
import type { AgentContext } from "./useAgentContext";
import type { SessionManager } from "./useSessionManager";
import { appSessionApi, projectAgentApi } from "../lib/tauriApi";

export function useSessionAgentSelection(
  sessions: SessionManager,
  agents: AgentContext,
  showToast: (message: string, severity?: "error" | "info") => void,
  enabled: () => boolean = () => true,
) {
  let selection = 0;
  const pendingSelections = new Map<string, Promise<void>>();
  createEffect(() => {
    const session = sessions.activeSession();
    const id = session?.id;
    const role = session?.activeRole;
    const runtime = session?.runtimeKind;
    if (!enabled() || !id || !role || !runtime) return;
    let disposed = false;
    onCleanup(() => { disposed = true; });
    void agents.fetchConfigOptions(runtime, role).then((options) => {
      const current = sessions.activeSession();
      if (!disposed && current?.id === id && current.activeRole === role && current.runtimeKind === runtime) {
        sessions.updateSession(id, { discoveredConfigOptions: options });
      }
    });
  });

  const selectAgent = (runtimeKind: string, runtimeProfileId: string | null) => {
    ++selection;
    sessions.patchActiveSession({ runtimeKind, runtimeProfileId, discoveredConfigOptions: [] });
  };

  const selectRole = async (roleName: string) => {
    const session = sessions.activeSession();
    if (!session) return;
    const id = session.id;
    const request = ++selection;
    if (session.persisted) {
      const previous = pendingSelections.get(id) ?? Promise.resolve();
      const next = previous.catch(() => {}).then(async () => {
        if (selection !== request) return;
        const selected = await appSessionApi.selectRole(id, roleName);
        if (selection !== request || sessions.activeSessionId() !== id) return;
        sessions.updateSession(id, {
          activeRole: selected.activeRole,
          runtimeKind: selected.runtimeKind,
          runtimeProfileId: selected.runtimeProfileId,
          discoveredConfigOptions: [],
        });
      });
      pendingSelections.set(id, next);
      try {
        await next;
      } catch (error) {
        if (selection === request) showToast(`Could not switch persona: ${String(error)}`, "error");
      } finally {
        if (pendingSelections.get(id) === next) pendingSelections.delete(id);
      }
      return;
    }
    try {
      const config = await projectAgentApi.getConfig(session.projectId ?? null, roleName);
      if (selection !== request || sessions.activeSessionId() !== id) return;
      const role = agents.roles().find((role) => role.roleName === roleName);
      const runtime = config.runtimeKind ?? role?.runtimeKind ?? session.runtimeKind;
      const assistant = agents.assistants().find((assistant) => assistant.key === runtime);
      sessions.patchActiveSession({
        activeRole: roleName,
        runtimeKind: runtime,
        runtimeProfileId: assistant?.profileId ?? role?.runtimeProfileId ?? null,
        discoveredConfigOptions: [],
      });
    } catch (error) {
      if (selection === request) showToast(`Could not switch persona: ${String(error)}`, "error");
    }
  };

  return { selectAgent, selectRole };
}
