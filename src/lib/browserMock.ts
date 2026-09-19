/**
 * Browser mock for Tauri IPC internals.
 * Automatically activates when running in standard web browsers (e.g. `pnpm dev` in Chrome/Firefox)
 * so developers can iterate on UI, themes, slash commands, and interactions without the Rust backend.
 */

if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
  console.info(
    "%c[Jockey Standalone]%c Running in browser mode with mock IPC bridge. Real desktop features are simulated.",
    "background: #3b82f6; color: white; padding: 2px 6px; border-radius: 4px; font-weight: bold;",
    "color: #60a5fa;"
  );

  const STORAGE_KEY_PROJECTS = "jockey_mock_projects";
  const STORAGE_KEY_SESSIONS = "jockey_mock_sessions";
  const STORAGE_KEY_ROLES = "jockey_mock_roles";

  type MockProject = {
    id: string;
    name: string;
    rootPath: string;
    createdAt: number;
    updatedAt: number;
  };
  type MockMessage = { id: string; roleName: string; text: string; at: number };
  type MockSession = {
    id: string;
    title: string;
    projectId: string | null;
    activeRole: string;
    runtimeKind: string;
    runtimeProfileId: string;
    cwd: string | null;
    messages: MockMessage[];
    createdAt: number;
    lastActiveAt: number;
    closedAt?: number | null;
  };
  type MockRole = {
    id: string;
    name: string;
    roleName: string;
    runtimeKind: string;
    prompt: string;
    systemPrompt: string;
    skills: string[];
    projectId: string | null;
    isBuiltin: boolean;
    createdAt: number;
    updatedAt: number;
  };

  const getStorage = <T>(key: string, defaultVal: T): T => {
    try {
      const item = localStorage.getItem(key);
      return item ? JSON.parse(item) : defaultVal;
    } catch {
      return defaultVal;
    }
  };

  const setStorage = <T>(key: string, val: T): void => {
    try {
      localStorage.setItem(key, JSON.stringify(val));
    } catch {
      // ignore
    }
  };

  if (!localStorage.getItem(STORAGE_KEY_PROJECTS)) {
    setStorage(STORAGE_KEY_PROJECTS, [
      {
        id: "proj-jockey",
        name: "jockey",
        rootPath: "/Users/sexy/Documents/GitHub/jockey",
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
    ]);
  }

  if (!localStorage.getItem(STORAGE_KEY_SESSIONS)) {
    setStorage(STORAGE_KEY_SESSIONS, [
      {
        id: "s-jockey-dev",
        title: "jockey",
        projectId: "proj-jockey",
        activeRole: "Developer",
        runtimeKind: "claude-native",
        runtimeProfileId: "claude-native",
        cwd: "/Users/sexy/Documents/GitHub/jockey",
        messages: [
          {
            id: "msg-welcome",
            roleName: "Developer",
            text: "👋 Welcome to **Jockey Standalone Browser Mode**!\n\nYou are running the SolidJS frontend directly in your browser. All slash completions, session chips, sidebar panels, and role configs work with live mock IPC.",
            at: Date.now(),
          },
        ],
        createdAt: Date.now() - 3600000,
        lastActiveAt: Date.now(),
      },
      {
        id: "s-jockey-review",
        title: "code-review",
        projectId: "proj-jockey",
        activeRole: "Reviewer",
        runtimeKind: "codex-cli",
        runtimeProfileId: "codex-cli",
        cwd: "/Users/sexy/Documents/GitHub/jockey",
        messages: [],
        createdAt: Date.now() - 1800000,
        lastActiveAt: Date.now() - 600000,
      },
    ]);
  }

  if (!localStorage.getItem(STORAGE_KEY_ROLES)) {
    setStorage(STORAGE_KEY_ROLES, [
      {
        id: "r-developer",
        name: "Developer",
        roleName: "Developer",
        runtimeKind: "claude-native",
        prompt: "",
        systemPrompt: "",
        skills: ["review"],
        isBuiltin: true,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
      {
        id: "r-architect",
        name: "Architect",
        roleName: "Architect",
        runtimeKind: "codex-cli",
        prompt: "",
        systemPrompt: "",
        skills: [],
        isBuiltin: true,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
    ]);
  }

  const callbacks = new Map<number, (res: unknown) => void>();
  let nextCallbackId = 1;

  const eventListeners = new Map<string, Map<number, (payload: unknown) => void>>();
  let nextListenerId = 1;

  (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, eventId: number) => {
      const listeners = eventListeners.get(event);
      if (listeners) {
        listeners.delete(eventId);
      }
    },
  };

  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    transformCallback: (callback: (res: unknown) => void, once = false) => {
      const id = nextCallbackId++;
      callbacks.set(id, (res) => {
        if (once) callbacks.delete(id);
        callback(res);
      });
      return id;
    },
    unregisterCallback: (id: number) => {
      callbacks.delete(id);
    },
    convertFileSrc: (filePath: string) => filePath,
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      await new Promise((r) => setTimeout(r, 15));

      switch (cmd) {
        case "list_projects_cmd": {
          return getStorage<MockProject[]>(STORAGE_KEY_PROJECTS, []);
        }
        case "get_project_cmd": {
          const list = getStorage<MockProject[]>(STORAGE_KEY_PROJECTS, []);
          return list.find((p: { id: string }) => p.id === args.id) ?? null;
        }
        case "create_project_cmd": {
          const list = getStorage<MockProject[]>(STORAGE_KEY_PROJECTS, []);
          const created = {
            id: `proj-${Date.now()}`,
            name: String(args.name || "Untitled"),
            rootPath: String(args.rootPath || "/tmp"),
            createdAt: Date.now(),
            updatedAt: Date.now(),
          };
          list.push(created);
          setStorage(STORAGE_KEY_PROJECTS, list);
          return created;
        }
        case "delete_project_cmd": {
          let list = getStorage<MockProject[]>(STORAGE_KEY_PROJECTS, []);
          list = list.filter((p: { id: string }) => p.id !== args.id);
          setStorage(STORAGE_KEY_PROJECTS, list);
          return;
        }
        case "import_project_sessions_cmd": {
          return [];
        }
        case "scan_importable_sessions_cmd": {
          return [];
        }

        case "list_app_sessions": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          return list.filter((s: { closedAt?: number | null }) => !s.closedAt);
        }
        case "list_closed_app_sessions": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          return list.filter((s: { closedAt?: number | null }) => !!s.closedAt);
        }
        case "reopen_app_session": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          const s = list.find((item: { id: string }) => item.id === args.id);
          if (s) {
            s.closedAt = null;
            s.lastActiveAt = Date.now();
            setStorage(STORAGE_KEY_SESSIONS, list);
            return s;
          }
          return null;
        }
        case "create_app_session": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          const created = {
            id: `s-${Date.now()}`,
            title: String(args.title || "New Session"),
            projectId: typeof args.projectId === "string" ? args.projectId : null,
            activeRole: "Developer",
            runtimeKind: "claude-native",
            runtimeProfileId: "claude-native",
            cwd: null,
            messages: [],
            createdAt: Date.now(),
            lastActiveAt: Date.now(),
          };
          list.unshift(created);
          setStorage(STORAGE_KEY_SESSIONS, list);
          return created;
        }
        case "update_app_session": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          const idx = list.findIndex((s: { id: string }) => s.id === args.id);
          if (idx !== -1) {
            const updates = (args.updates as Record<string, unknown>) || {};
            list[idx] = { ...list[idx], ...updates, lastActiveAt: Date.now() };
            setStorage(STORAGE_KEY_SESSIONS, list);
            return list[idx];
          }
          return null;
        }
        case "delete_app_session": {
          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          const s = list.find((item: { id: string }) => item.id === args.id);
          if (s) {
            s.closedAt = Date.now();
            setStorage(STORAGE_KEY_SESSIONS, list);
          }
          return;
        }

        case "list_roles": {
          return getStorage<MockRole[]>(STORAGE_KEY_ROLES, []);
        }
        case "list_app_skills": {
          return [];
        }
        case "upsert_role_cmd": {
          const list = getStorage<MockRole[]>(STORAGE_KEY_ROLES, []);
          const input = (args.input || {}) as Record<string, unknown>;
          const roleName = String(input.roleName || input.name || "Role");
          const targetId = input.id ? String(input.id) : null;
          const inputPid = input.projectId ? String(input.projectId) : null;
          const existingIdx = list.findIndex(
            (r: { id?: string; roleName: string; projectId?: string | null }) =>
              (targetId && r.id === targetId) ||
              (r.roleName.toLowerCase() === roleName.toLowerCase() &&
                ((r.projectId || null) === inputPid))
          );
          const roleObj = {
            id: targetId || (existingIdx !== -1 ? list[existingIdx].id : `r-${Date.now()}`),
            name: roleName,
            roleName,
            runtimeKind: String(input.runtimeKind || "claude-native"),
            prompt: String(input.prompt || ""),
            systemPrompt: String(input.systemPrompt || ""),
            skills: Array.isArray(input.skills) ? input.skills.map(String) : [],
            projectId: inputPid,
            isBuiltin: false,
            createdAt: Date.now(),
            updatedAt: Date.now(),
          };
          if (existingIdx !== -1) {
            list[existingIdx] = roleObj;
          } else {
            list.push(roleObj);
          }
          setStorage(STORAGE_KEY_ROLES, list);
          return roleObj;
        }
        case "delete_role_cmd": {
          let list = getStorage<MockRole[]>(STORAGE_KEY_ROLES, []);
          list = list.filter(
            (r: { id?: string; roleName: string; projectId?: string | null }) =>
              args.roleId
                ? r.id !== args.roleId
                : args.projectId !== undefined
                ? !(r.roleName === args.roleName && (r.projectId || null) === (args.projectId || null))
                : r.roleName !== args.roleName
          );
          setStorage(STORAGE_KEY_ROLES, list);
          return;
        }

        case "detect_assistants": {
          const caps = { mcpServers: true, sessionRestore: true, dynamicModes: true, modelCatalog: true, rewind: false, fork: false };
          return [
            { key: "claude-native", name: "Claude Code", label: "Claude Code", family: "native", available: true, profileId: "claude-native", transport: "stdio", capabilities: caps },
            { key: "codex-cli", name: "Codex CLI", label: "Codex CLI", family: "native", available: true, profileId: "codex-cli", transport: "stdio", capabilities: caps },
            { key: "antigravity-cli", name: "Antigravity CLI", label: "Antigravity CLI", family: "native", available: true, profileId: "antigravity-cli", transport: "stdio", capabilities: caps },
            { key: "pi-cli", name: "Pi CLI", label: "Pi CLI", family: "native", available: true, profileId: "pi-cli", transport: "stdio", capabilities: caps },
            { key: "mock", name: "Mock Assistant", label: "Mock Assistant", family: "native", available: true, profileId: "mock", transport: "in-process", capabilities: caps },
          ];
        }
        case "list_available_commands_cmd": {
          return [
            { name: "compact", description: "Compact session conversation context", role: null, source: "built-in" },
            { name: "clear", description: "Clear current session messages", role: null, source: "built-in" },
            { name: "cost", description: "Show token usage & estimated cost", role: null, source: "built-in" },
            { name: "review", description: "Run automated code review on current diff", role: null, source: "skill" },
            { name: "test", description: "Execute test suite and analyze failures", role: null, source: "skill" },
          ];
        }
        case "list_discovered_modes_cmd": {
          return ["code", "architect", "ask"];
        }
        case "list_discovered_config_options_cmd": {
          return [];
        }
        case "prewarm_role_config_cmd": {
          return { configOptions: [], modes: ["code", "architect", "ask"] };
        }

        case "assistant_chat": {
          const chatInput = (args.input || {}) as Record<string, unknown>;
          const text = String(chatInput.input || "");
          const sessionId = String(chatInput.appSessionId || "");
          const isSlash = text.trim().startsWith("/");

          let reply = `Echo: Received "${text}". (Browser Standalone Mode)`;
          if (isSlash) {
            reply = `Executed mock command \`${text.trim()}\` successfully.`;
          }

          const list = getStorage<MockSession[]>(STORAGE_KEY_SESSIONS, []);
          const s = list.find((item: { id: string }) => item.id === sessionId);
          if (s) {
            s.messages = s.messages || [];
            s.messages.push({
              id: `msg-${Date.now()}`,
              roleName: s.activeRole || "Developer",
              text: reply,
              at: Date.now(),
            });
            setStorage(STORAGE_KEY_SESSIONS, list);
          }

          return {
            ok: true,
            reply,
            runtimeKind: s?.runtimeKind ?? "claude-native",
            sessionId,
            commandResult: null,
          };
        }
        case "apply_chat_command": {
          const cmdText = String(args.input || "");
          return {
            ok: true,
            message: `Mock applied command: ${cmdText}`,
            runtimeKind: null,
            sessionId: args.appSessionId ?? null,
            payload: {},
          };
        }

        case "git_status_cmd": {
          return {
            branch: "main",
            clean: true,
            ahead: 0,
            behind: 0,
            staged: [],
            unstaged: [],
            untracked: [],
          };
        }
        case "git_diff_cmd": {
          return "";
        }
        case "open_workspace_cmd": {
          console.log("[Mock Workspace] open_workspace_cmd:", args);
          return;
        }
        case "get_workspace_app_icon_cmd": {
          return "";
        }

        case "plugin:opener|open_url": {
          const url = (args.url as string) || (args.path as string);
          if (url) window.open(url, "_blank");
          return;
        }
        case "plugin:dialog|open": {
          return "/Users/sexy/Documents/GitHub/jockey";
        }
        case "plugin:event|listen": {
          const eventName = args.event as string;
          const handlerId = args.handler as number;
          if (!eventListeners.has(eventName)) {
            eventListeners.set(eventName, new Map());
          }
          const listenerId = nextListenerId++;
          eventListeners.get(eventName)!.set(listenerId, (payload) => {
            const cb = callbacks.get(handlerId);
            if (cb) cb({ event: eventName, id: listenerId, payload });
          });
          return listenerId;
        }
        case "plugin:event|unlisten": {
          const eventName = args.event as string;
          const eventId = args.eventId as number;
          eventListeners.get(eventName)?.delete(eventId);
          return;
        }
        case "plugin:event|emit":
        case "plugin:event|emit_to": {
          const eventName = args.event as string;
          const payload = args.payload;
          const listeners = eventListeners.get(eventName);
          if (listeners) {
            listeners.forEach((fn) => fn(payload));
          }
          return;
        }

        default: {
          console.debug(`[Tauri Mock IPC] Unhandled cmd: ${cmd}`, args);
          return null;
        }
      }
    },
  };
}
