import type { Accessor } from "solid-js";
import { DEFAULT_ROLE_ALIAS } from "../components/types";
import type { AppSession, AssistantRuntime, QueuedItem, Role } from "../components/types";
import { makeDefaultSession, makeDraftSession } from "../lib/sessionHelpers";
import { appSessionApi, projectAgentApi } from "../lib/tauriApi";

type SetSessions = {
  (value: AppSession[]): void;
  (index: number, key: "runtimeKind", value: string | null): void;
  (index: number, key: "runtimeProfileId", value: string | null): void;
  (index: number, key: "queuedItems", value: QueuedItem[]): void;
};

type UseAppBootstrapInput = {
  setSessions: SetSessions;
  setActiveSessionId: (id: string) => void;
  assistants: Accessor<AssistantRuntime[]>;
  roles: Accessor<Role[]>;
  currentProjectId?: Accessor<string | undefined>;
  refreshAssistants: () => Promise<void>;
  refreshRoles: (projectId?: string) => Promise<void>;
  refreshSkills: () => Promise<void>;
  fetchConfigOptions: (runtimeKey: string, roleName?: string) => Promise<unknown[]>;
  showToast: (message: string, severity?: "error" | "info") => void;
};

export function useAppBootstrap(input: UseAppBootstrapInput) {
  const {
    setSessions,
    setActiveSessionId,
    assistants,
    roles,
    currentProjectId,
    refreshAssistants,
    refreshRoles,
    refreshSkills,
    fetchConfigOptions,
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
    await Promise.all(loaded.map(async (session, index) => {
      if (!session.persisted) return;
      try {
        const inbox = await appSessionApi.listInbox(session.id);
        setSessions(index, "queuedItems", inbox.map((item) => ({
          id: item.id,
          clientId: item.id,
          text: item.text,
          attachments: item.attachments ?? [],
          roleName: item.roleName,
          delivery: item.delivery === "nextStep" ? "nextStep" : "nextTurn",
          createdAt: item.createdAt,
          status: "queued",
        })));
      } catch {
        // A missing inbox table on an older development database must not block session restore.
      }
    }));
    await Promise.all([refreshAssistants(), refreshRoles(pid), refreshSkills()]);

    const available = assistants().filter((a) => a.available);
    for (let i = 0; i < loaded.length; i++) {
      const sessionRole = loaded[i].activeRole || DEFAULT_ROLE_ALIAS;
      const sessionPid = loaded[i].projectId || pid || null;
      let pinned: string | null = null;
      try {
        const cfg = await projectAgentApi.getConfig(sessionPid, sessionRole);
        pinned = cfg.runtimeKind;
      } catch {
        // Ignore lookup failure and fall back to persona default
      }
      const personaDefault = roles().find((r) => r.roleName === sessionRole)?.runtimeKind ?? null;
      const chosen =
        available.find((a) => a.key === pinned) ??
        available.find((a) => a.key === personaDefault) ??
        available.find((a) => a.key === loaded[i].runtimeKind) ??
        available[0] ??
        null;

      if (chosen && chosen.key !== loaded[i].runtimeKind) {
        loaded[i].runtimeKind = chosen.key;
        loaded[i].runtimeProfileId = chosen.profileId;
        setSessions(i, "runtimeKind", chosen.key);
        setSessions(i, "runtimeProfileId", chosen.profileId);
        if (loaded[i].persisted) {
          void appSessionApi
            .update(loaded[i].id, {
              runtimeKind: chosen.key,
              runtimeProfileId: chosen.profileId,
            })
            .catch(() => {});
        }
      }
    }

    const targetSession = pid ? loaded.find((s) => s.projectId === pid) : null;
    setActiveSessionId(targetSession?.id ?? loaded[0].id);
  };

  return { bootstrapApp };
}
