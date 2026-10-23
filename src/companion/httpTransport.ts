import type { Transport, UnlistenFn } from "../lib/transport";

const TOKEN_KEY = "jockey_token";

/** Events the gateway relays: the engine's lifecycle events plus the two stream frames. */
const RELAYED_EVENTS = [
  "turn/started",
  "turn/finished",
  "turn/progress",
  "queue/changed",
  "message/added",
  "session/changed",
  "project/changed",
  "acp/delta",
  "acp/stream",
  "permission_update",
] as const;

export class UnauthorizedError extends Error {
  constructor() {
    super("需要重新输入 Token");
  }
}

type Handler = (ev: { payload: unknown }) => void;
type Args = Record<string, unknown>;

export function readStoredToken(): string {
  const fromUrl = new URLSearchParams(window.location.search).get("token");
  try {
    if (fromUrl) {
      window.localStorage.setItem(TOKEN_KEY, fromUrl);
      // Keep the token out of browser history, bookmarks and screenshots.
      window.history.replaceState(null, "", window.location.pathname);
      return fromUrl;
    }
    return window.localStorage.getItem(TOKEN_KEY) ?? "";
  } catch {
    return fromUrl ?? "";
  }
}

export function storeToken(token: string) {
  try {
    window.localStorage.setItem(TOKEN_KEY, token);
  } catch { /* private mode: the token lives for this page only */ }
}

export function clearToken() {
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch { /* nothing stored */ }
}

type RawSummary = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  projectId: string | null;
  cwd: string | null;
  lastActiveAt: number;
  closedAt: number | null;
};

const summaryToRaw = (s: RawSummary) => ({
  id: s.id,
  title: s.title,
  activeRole: s.activeRole,
  runtimeKind: s.runtimeKind,
  projectId: s.projectId,
  cwd: s.cwd,
  lastActiveAt: s.lastActiveAt,
  closedAt: s.closedAt,
  messages: [],
});

export type HttpTransport = Transport & {
  /** The stream follows this session; frames for other sessions are not delivered. */
  setFocus(sessionId: string | null): void;
  onResync(cb: () => void): () => void;
  onUnauthorized(cb: () => void): void;
  setToken(token: string): void;
  hasToken(): boolean;
};

export function createHttpTransport(initialToken: string): HttpTransport {
  let token = initialToken;
  let focus: string | null = null;
  let source: EventSource | null = null;
  let hadError = false;
  let unauthorized: () => void = () => {};
  const handlers = new Map<string, Set<Handler>>();
  const resyncs = new Set<() => void>();

  const api = async <T>(path: string, init?: { method?: string; body?: unknown }): Promise<T> => {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 30_000);
    let res: Response;
    try {
      res = await fetch(`/api${path}`, {
        signal: controller.signal,
        method: init?.method ?? (init?.body === undefined ? "GET" : "POST"),
        headers: {
          Authorization: `Bearer ${token}`,
          ...(init?.body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: init?.body === undefined ? undefined : JSON.stringify(init.body),
      });
    } finally {
      clearTimeout(timeout);
    }
    if (res.status === 401) {
      clearToken();
      token = "";
      source?.close();
      source = null;
      unauthorized();
      throw new UnauthorizedError();
    }
    const body = await res.json().catch(() => ({}));
    if (!res.ok || (body && body.ok === false)) {
      throw new Error((body && body.error) || `HTTP ${res.status}`);
    }
    return body as T;
  };

  const q = (params: Record<string, string | undefined | null>) => {
    const entries = Object.entries(params).filter(([, v]) => v) as [string, string][];
    return entries.length ? `?${new URLSearchParams(entries)}` : "";
  };
  const enc = encodeURIComponent;

  const dispatch = (event: string, payload: unknown) => {
    handlers.get(event)?.forEach((h) => h({ payload }));
  };

  const connect = () => {
    source?.close();
    if (!token || handlers.size === 0) {
      source = null;
      return;
    }
    const es = new EventSource(`/api/events${q({ token, focus })}`);
    source = es;
    for (const name of RELAYED_EVENTS) {
      es.addEventListener(name, (ev) => {
        try {
          dispatch(name, JSON.parse((ev as MessageEvent<string>).data));
        } catch { /* a malformed frame is skipped; the next one or a resync recovers */ }
      });
    }
    es.addEventListener("resync", () => resyncs.forEach((cb) => cb()));
    es.onerror = () => { hadError = true; };
    es.onopen = () => {
      if (hadError) {
        hadError = false;
        resyncs.forEach((cb) => cb());
      }
    };
  };

  const call = async <T>(command: string, args: Args = {}): Promise<T> => {
    const a = args as Record<string, any>;
    switch (command) {
      case "list_projects_cmd":
        return api("/projects");
      case "get_project_cmd": {
        const all = await api<Array<{ id: string }>>("/projects");
        return (all.find((p) => p.id === a.id) ?? null) as T;
      }
      case "create_project_cmd":
        return api("/projects", { body: { name: a.name, rootPath: a.rootPath } });
      case "scan_importable_sessions_cmd":
        return api(`/projects/${enc(a.projectId)}/importable${q({ refresh: a.forceRefresh ? "true" : undefined })}`);
      case "scan_all_importable_sessions_cmd":
        return api(`/importable${q({ refresh: a.forceRefresh ? "true" : undefined })}`);
      case "import_cli_sessions_cmd": {
        const summaries = await api<RawSummary[]>("/import", { body: { sourceIds: a.sourceIds } });
        return { sessions: summaries.map(summaryToRaw), createdProjects: [] } as T;
      }
      case "import_project_sessions_cmd": {
        const summaries = await api<RawSummary[]>(`/projects/${enc(a.projectId)}/import`, {
          body: { externalSessionIds: a.externalSessionIds ?? [] },
        });
        return summaries.map(summaryToRaw) as T;
      }
      case "list_app_sessions": {
        const rows = await api<RawSummary[]>(`/sessions${q({ projectId: a.projectId })}`);
        return rows.map(summaryToRaw) as T;
      }
      case "list_session_summaries_cmd":
        return api(`/sessions${a.closed ? "/closed" : ""}${q({ projectId: a.projectId })}`);
      case "list_closed_app_sessions": {
        const rows = await api<RawSummary[]>("/sessions/closed");
        return rows.map(summaryToRaw) as T;
      }
      case "get_app_sessions":
        return Promise.all((a.ids as string[]).map((id) => api(`/sessions/${enc(id)}?tail=200`))) as Promise<T>;
      case "create_app_session":
        return api("/sessions", { body: { title: a.title, projectId: a.projectId } });
      case "reopen_app_session": {
        await api(`/sessions/${enc(a.id)}/reopen`, { body: {} });
        return api(`/sessions/${enc(a.id)}?tail=200`);
      }
      case "update_app_session": {
        const role = (a.update as { activeRole?: string }).activeRole;
        if (role) await api(`/sessions/${enc(a.id)}/role`, { body: { role } });
        return undefined as T;
      }
      case "delete_app_session":
        return api(`/sessions/${enc(a.id)}/close`, { body: {} });
      case "watch_session":
      case "get_live_turn":
        return api(`/sessions/${enc(a.id)}/live`);
      case "unwatch_session":
        return undefined as T;
      case "list_session_inbox_cmd":
        return api(`/sessions/${enc(a.sessionId)}/queue`);
      case "submit_input":
        return api(`/sessions/${enc(a.input.sessionId)}/chat`, {
          body: { ...a.input, input: a.input.text },
        });
      case "cancel_turn": {
        const res = await api<{ cancelled: boolean }>(`/sessions/${enc(a.sessionId)}/cancel`, { body: {} });
        return res.cancelled as T;
      }
      case "send_queued_input_now":
        return api(`/sessions/${enc(a.sessionId)}/send-now`, { body: {} });
      case "remove_queued_input":
        return api(`/sessions/${enc(a.sessionId)}/queue/${enc(a.id)}/remove`, { body: {} });
      case "list_roles": {
        const rows = await api<Array<{ roleName: string; runtimeKind: string; projectScoped: boolean }>>(
          `/roles${q({ projectId: a.projectId })}`,
        );
        return rows.map((r) => ({ id: r.roleName, name: r.roleName, skills: [], ...r })) as T;
      }
      case "detect_assistants":
        return api("/assistants");
      case "list_pending_prompts_cmd":
        return api(`/pending${q({ sessionId: a.sessionId })}`);
      case "respond_permission":
        return api("/permissions/respond", { body: a });
      case "respond_user_input_cmd":
        return api("/user-input/respond", { body: a });
      case "reconnect_acp_session":
        return api(`/sessions/${enc(a.appSessionId)}/reconnect`, { body: { role: a.roleName } });
      case "reset_acp_session":
        return api(`/sessions/${enc(a.appSessionId)}/reset`, { body: { role: a.roleName } });
      // Local notices are not engine turns; the phone shows them without writing them.
      case "save_app_message":
      case "append_app_message":
        return undefined as T;
      case "list_app_skills":
      case "list_available_commands_cmd":
        return [] as T;
      case "prewarm_role_config_cmd":
        return { configOptions: [], modes: [] } as T;
      case "gateway_list_dirs":
        return api(`/fs/dirs${q({ path: a.path })}`);
      case "gateway_list_sessions": {
        return api(`/sessions${q({ projectId: a.projectId })}`);
      }
      case "gateway_list_closed_sessions":
        return api(`/sessions/closed${q({ projectId: a.projectId })}`);
      default:
        throw new Error(`"${command}" is not available from the phone companion`);
    }
  };

  const listen = async <T,>(event: string, handler: (ev: { payload: T }) => void): Promise<UnlistenFn> => {
    const set = handlers.get(event) ?? new Set<Handler>();
    set.add(handler as Handler);
    handlers.set(event, set);
    if (!source) connect();
    return () => {
      set.delete(handler as Handler);
      if (set.size === 0) handlers.delete(event);
      if (handlers.size === 0) {
        source?.close();
        source = null;
      }
    };
  };

  return {
    call,
    listen,
    openExternal: (url) => {
      window.open(url, "_blank", "noopener,noreferrer");
    },
    setFocus(sessionId) {
      if (focus === sessionId) return;
      focus = sessionId;
      connect();
    },
    onResync(cb) {
      resyncs.add(cb);
      return () => resyncs.delete(cb);
    },
    onUnauthorized(cb) {
      unauthorized = cb;
    },
    setToken(next) {
      token = next;
      storeToken(next);
      connect();
    },
    hasToken: () => token.length > 0,
  };
}
