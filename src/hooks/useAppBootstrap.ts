import type { Accessor } from "solid-js";
import type { AppSession, AssistantRuntime } from "../components/types";
import { makeDefaultSession, makeDraftSession } from "../lib/sessionHelpers";
import { appSessionApi } from "../lib/tauriApi";

type SetSessions = {
  (value: AppSession[]): void;
  (index: number, key: "runtimeKind", value: string | null): void;
  (index: number, key: "runtimeProfileId", value: string | null): void;
};

type UseAppBootstrapInput = {
  setSessions: SetSessions;
  setActiveSessionId: (id: string) => void;
  assistants: Accessor<AssistantRuntime[]>;
  currentProjectId?: Accessor<string | undefined>;
  refreshAssistants: () => Promise<void>;
  refreshRoles: (projectId?: string) => Promise<void>;
  refreshSkills: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<unknown[]>;
  pushMessage: (role: string, text: string) => void;
  showToast: (message: string, severity?: "error" | "info") => void;
};

export function useAppBootstrap(input: UseAppBootstrapInput) {
  const {
    setSessions,
    setActiveSessionId,
    assistants,
    currentProjectId,
    refreshAssistants,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
    pushMessage,
    showToast,
  } = input;

  const bootstrapApp = async () => {
    let loaded: AppSession[] = [];
    const pid = currentProjectId?.();
    try {
      const raw = await appSessionApi.list();
      loaded = raw.map((r) => {
        const s = makeDefaultSession(r.title);
        s.id = r.id;
        if (r.activeRole) s.activeRole = r.activeRole;
        if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
        if (r.runtimeProfileId !== undefined) s.runtimeProfileId = r.runtimeProfileId;
        if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
        if (r.projectId !== undefined) s.projectId = r.projectId ?? null;
        s.messages = r.messages ?? [];
        return s;
      });
    } catch (e) {
      showToast(`Failed to restore sessions: ${String(e)}`);
    }

    if (loaded.length === 0) {
      // No DB row yet — created lazily on the first real send (see `ensureSessionPersisted`),
      // so a fresh install / empty project doesn't spawn anything before the user does.
      loaded = [makeDraftSession("Session_1", { projectId: pid ?? null })];
    }

    setSessions(loaded);
    const targetSession = pid ? loaded.find((s) => s.projectId === pid) : null;
    setActiveSessionId(targetSession?.id ?? loaded[0].id);

    await Promise.all([refreshAssistants(), refreshRoles(pid), refreshSkills()]);

    const availableAssistant = assistants().find((a) => a.available) ?? null;
    for (let i = 0; i < loaded.length; i++) {
      if (!loaded[i].runtimeKind && availableAssistant) {
        setSessions(i, "runtimeKind", availableAssistant.key);
        setSessions(i, "runtimeProfileId", availableAssistant.profileId);
        void appSessionApi
          .update(loaded[i].id, {
            runtimeKind: availableAssistant.key,
            runtimeProfileId: availableAssistant.profileId,
          })
          .catch(() => {});
      }
    }

    assistants()
      .filter((a) => a.available)
      .forEach((a) => {
        void fetchConfigOptions(a.key);
      });
  };

  return { bootstrapApp };
}
