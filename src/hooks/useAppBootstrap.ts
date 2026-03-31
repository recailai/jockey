import type { Accessor } from "solid-js";
import type { AppSession, AssistantRuntime } from "../components/types";
import { makeDefaultSession } from "../lib/sessionHelpers";
import { appSessionApi } from "../lib/tauriApi";

type SetSessions = {
  (value: AppSession[]): void;
  (index: number, key: "runtimeKind", value: string | null): void;
};

type UseAppBootstrapInput = {
  setSessions: SetSessions;
  setActiveSessionId: (id: string) => void;
  assistants: Accessor<AssistantRuntime[]>;
  refreshAssistants: () => Promise<void>;
  refreshRoles: () => Promise<void>;
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
    refreshAssistants,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
    pushMessage,
    showToast,
  } = input;

  const bootstrapApp = async () => {
    let loaded: AppSession[] = [];
    try {
      const raw = await appSessionApi.list();
      loaded = raw.map((r) => {
        const s = makeDefaultSession(r.title);
        s.id = r.id;
        if (r.activeRole) s.activeRole = r.activeRole;
        if (r.runtimeKind !== undefined) s.runtimeKind = r.runtimeKind;
        if (r.cwd !== undefined) s.cwd = r.cwd ?? null;
        s.messages = r.messages ?? [];
        return s;
      });
    } catch (e) {
      showToast(`Failed to restore sessions: ${String(e)}`);
    }

    if (loaded.length === 0) {
      try {
        const created = await appSessionApi.create("Session_1");
        const s = makeDefaultSession("Session_1");
        s.id = created.id;
        loaded = [s];
      } catch {
        loaded = [makeDefaultSession("Session_1")];
      }
    }

    setSessions(loaded);
    setActiveSessionId(loaded[0].id);

    await Promise.all([refreshAssistants(), refreshRoles(), refreshSkills()]);

    const availableAssistant = assistants().find((a) => a.available)?.key ?? null;
    for (let i = 0; i < loaded.length; i++) {
      if (!loaded[i].runtimeKind && availableAssistant) {
        setSessions(i, "runtimeKind", availableAssistant);
        void appSessionApi
          .update(loaded[i].id, { runtimeKind: availableAssistant })
          .catch(() => {});
      }
    }

    pushMessage("system", "Welcome to JockeyUI. Agent sessions are warming up in the background.");

    assistants()
      .filter((a) => a.available)
      .forEach((a) => {
        void fetchConfigOptions(a.key);
      });
  };

  return { bootstrapApp };
}
