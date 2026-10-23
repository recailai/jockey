import type { UnlistenFn } from "../lib/transport";
import type { HttpTransport } from "./httpTransport";

type Handler = (ev: { payload: unknown }) => void;

const T0 = Date.now();
const min = (n: number) => T0 - n * 60_000;

const projects = [
  { id: "p1", name: "jockey", rootPath: "/Users/dev/GitHub/jockey", createdAt: T0, updatedAt: T0 },
  { id: "p2", name: "notes", rootPath: "/Users/dev/Documents/notes", createdAt: T0, updatedAt: T0 },
];

type MockSession = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string;
  projectId: string;
  lastActiveAt: number;
  preview: string;
  running: boolean;
  messages: Array<Record<string, unknown>>;
};

const doneTool = (id: string, title: string, kind: string, status = "completed") => ({
  toolCallId: id,
  title,
  kind,
  status,
  roleName: "Developer",
});

const sessions: MockSession[] = [
  {
    id: "s-chat",
    title: "Fix gateway SSE reconnect",
    activeRole: "Developer",
    runtimeKind: "claude-native",
    projectId: "p1",
    lastActiveAt: min(3),
    preview: "The resync handler now reloads the focused session.",
    running: false,
    messages: [
      { id: "m1", roleName: "user", text: "Why does the phone miss events after the screen locks?", at: min(9) },
      {
        id: "m2",
        roleName: "Developer",
        text: "The `EventSource` closes while the tab sleeps. On wake it reconnects, but frames sent in between are gone, so the page must **resync**.\n\n- reload the focused session\n- pull the live turn\n- refetch pending prompts",
        at: min(8),
        toolCalls: [
          doneTool("t1", "Read src-tauri/src/web_gateway/api.rs", "read"),
          doneTool("t2", "grep resync src/lib", "search"),
        ],
        segments: [
          { kind: "tool", tc: doneTool("t1", "Read src-tauri/src/web_gateway/api.rs", "read"), roleName: "Developer" },
          { kind: "tool", tc: doneTool("t2", "grep resync src/lib", "search"), roleName: "Developer" },
          {
            kind: "text",
            text: "The `EventSource` closes while the tab sleeps. On wake it reconnects, but frames sent in between are gone, so the page must **resync**.\n\n- reload the focused session\n- pull the live turn\n- refetch pending prompts",
            roleName: "Developer",
          },
        ],
      },
    ],
  },
  {
    id: "s-stream",
    title: "Port companion to shared components",
    activeRole: "Developer",
    runtimeKind: "claude-native",
    projectId: "p1",
    lastActiveAt: min(0),
    preview: "Ready for your next direction.",
    running: false,
    messages: [{ id: "m10", roleName: "user", text: "Show me a streaming turn", at: min(1) }],
  },
  {
    id: "s-perm",
    title: "Run the test suite",
    activeRole: "Developer",
    runtimeKind: "codex-cli",
    projectId: "p1",
    lastActiveAt: min(12),
    preview: "Waiting for approval",
    running: true,
    messages: [{ id: "m20", roleName: "user", text: "Run cargo test and fix failures", at: min(13) }],
  },
  {
    id: "s-ask",
    title: "Pick a deploy target",
    activeRole: "Planner",
    runtimeKind: "claude-native",
    projectId: "p2",
    lastActiveAt: min(30),
    preview: "Question from the agent",
    running: true,
    messages: [{ id: "m30", roleName: "user", text: "Deploy the notes site", at: min(31) }],
  },
  {
    id: "s-error",
    title: "Nightly import job",
    activeRole: "Developer",
    runtimeKind: "claude-native",
    projectId: "p2",
    lastActiveAt: min(90),
    preview: "需要登录授权",
    running: false,
    messages: [
      { id: "m40", roleName: "user", text: "Import yesterday's transcripts", at: min(95) },
      { id: "m41", roleName: "event", text: "Background task completed: reindex notes", at: min(94) },
      { id: "m42", roleName: "event", text: "Background task failed: embed batch 3", at: min(93) },
      { id: "t9:error", roleName: "event", text: "Claude Code is not signed in. Run `claude login` and retry.", at: min(92), payload: { kind: "turnError", code: "AUTH_REQUIRED" } },
    ],
  },
];

const pending = {
  permissions: [
    {
      appSessionId: "s-chat",
      requestId: "perm-read",
      title: "Read repository configuration",
      description: "Read the project configuration before continuing the fix.",
      options: [
        { optionId: "read_once", title: "Allow once", kind: "allow_once" },
        { optionId: "deny", title: "Reject", kind: "reject_once" },
      ],
    },
    {
      appSessionId: "s-perm",
      requestId: "perm-1",
      title: "Run command: cargo test --manifest-path src-tauri/Cargo.toml",
      description: "The agent wants to execute a shell command in /Users/dev/GitHub/jockey.",
      options: [
        { optionId: "allow_once", title: "Allow once", kind: "allow_once" },
        { optionId: "allow_always", title: "Always allow cargo", kind: "allow_always" },
        { optionId: "reject_once", title: "Reject", kind: "reject_once" },
      ],
    },
  ],
  userInputs: [
    {
      appSessionId: "s-ask",
      requestId: "ask-1",
      title: "Deployment details",
      blocking: true,
      questions: [
        {
          id: "target",
          header: "Target",
          prompt: "Where should the notes site be deployed?",
          options: [
            { value: "staging", label: "Staging", description: "notes-staging.example.com" },
            { value: "prod", label: "Production", description: "notes.example.com" },
          ],
          allowOther: true,
          secret: false,
          multi: false,
        },
      ],
    },
  ],
};

const summary = (s: MockSession) => ({
  id: s.id,
  title: s.title,
  activeRole: s.activeRole,
  runtimeKind: s.runtimeKind,
  projectId: s.projectId,
  cwd: null,
  lastActiveAt: s.lastActiveAt,
  closedAt: null,
  messageCount: s.messages.length,
  preview: s.preview,
  lastError: s.id === "s-error" ? "Claude Code is not signed in. Open chat to reconnect after signing in." : null,
  running: s.running,
  externalActive: s.id === "s-chat" ? false : s.id === "s-ask",
});

const assistants = ["claude-native", "codex-cli"].map((key) => ({
  key,
  profileId: key,
  label: key,
  family: "native",
  binary: key,
  available: true,
  version: "1.0",
  installHint: null,
  unavailableReason: null,
  launchMethod: null,
  transport: "stdio",
  capabilities: { attachments: true },
  inputDelivery: { nextTurn: true, nextStep: true },
}));

const queued = [
  { id: "q1", appSessionId: "s-error", roleName: null, delivery: "nextTurn", text: "Retry the import after signing in", attachments: [], createdAt: min(1) },
];

export function createMockTransport(): HttpTransport {
  const handlers = new Map<string, Set<Handler>>();
  const emit = (event: string, payload: unknown) => handlers.get(event)?.forEach((h) => h({ payload }));
  const find = (id: string) => sessions.find((s) => s.id === id);
  const timers = new Map<string, number[]>();

  const streamTurn = (sid: string, input: Record<string, any>) => {
    const turnId = `turn-${Date.now()}`;
    const userMessage = { id: input.userMessageId, roleName: "user", text: input.text, at: Date.now() };
    const assistantMessageId = input.assistantMessageId;
    const session = find(sid)!;
    session.running = true;
    const base = { role: "Developer", appSessionId: sid, turnId, runtimeKind: "claude-native" };
    const stream = (event: Record<string, unknown>) => emit("acp/stream", { ...base, event });
    const delta = (text: string) => emit("acp/delta", { role: "Developer", appSessionId: sid, turnId, delta: text });
    emit("turn/started", {
      appSessionId: sid,
      turnId,
      assistantMessageId,
      roleLabel: "Developer",
      userMessage,
      session: {},
    });
    const steps: Array<() => void> = [
      () => stream({ kind: "thoughtDelta", text: "Checking which components touch the Tauri bridge before wiring the transport." }),
      () => stream({ kind: "toolCall", toolCallId: "c1", title: "Read src/lib/tauriApi.ts", toolKind: "read", status: "completed" }),
      () => stream({ kind: "toolCall", toolCallId: "c2", title: "Edit src/lib/transport.ts", toolKind: "edit", status: "completed" }),
      () => stream({ kind: "toolCall", toolCallId: "c3", title: "pnpm build", toolKind: "execute", status: "in_progress" }),
      () => delta("Every command now goes through a `Transport`, so the phone and the desktop run the **same** reducers.\n\n"),
      () => delta("Next I am mapping the SSE stream onto the existing `listen` handlers"),
    ];
    const scheduled = steps.map((step, i) => window.setTimeout(step, 150 + i * 120));
    scheduled.push(window.setTimeout(() => {
      const message = { id: assistantMessageId, roleName: "Developer", text: "The shared transport delivered this message. The agent is ready for your next direction.", at: Date.now() };
      session.messages.push(userMessage, message);
      session.running = false;
      session.preview = message.text;
      session.lastActiveAt = Date.now();
      emit("turn/finished", { appSessionId: sid, turnId, ok: true, cancelled: false, messages: [userMessage, message] });
      timers.delete(sid);
    }, 1100));
    timers.set(sid, scheduled);
  };

  const call = async (command: string, args: Record<string, any> = {}): Promise<any> => {
    switch (command) {
      case "list_projects_cmd": return projects;
      case "list_session_summaries_cmd":
      case "gateway_list_sessions": return sessions.map(summary);
      case "gateway_list_closed_sessions": return [];
      case "get_app_sessions":
        return (args.ids as string[]).map((id) => {
          const s = find(id)!;
          return { ...summary(s), messages: structuredClone(s.messages) };
        });
      case "watch_session":
      case "get_live_turn": return null;
      case "unwatch_session": return undefined;
      case "list_session_inbox_cmd": return queued.filter((q) => q.appSessionId === args.sessionId);
      case "detect_assistants": return assistants;
      case "list_roles": return [{ id: "Developer", name: "Developer", roleName: "Developer", runtimeKind: "claude-native", skills: [] }];
      case "list_pending_prompts_cmd": return structuredClone(pending);
      case "create_app_session": {
        const session = {
          id: `new-${sessions.length}`,
          title: args.title || "Session_1",
          activeRole: "Developer",
          runtimeKind: "claude-native",
          projectId: args.projectId,
          lastActiveAt: Date.now(),
          preview: "",
          running: false,
          messages: [],
        };
        sessions.push(session);
        return summary(session);
      }
      case "submit_input": {
        streamTurn(args.input.sessionId, args.input);
        return { status: "started" };
      }
      case "respond_permission":
      case "respond_user_input_cmd": {
        if (new URLSearchParams(window.location.search).has("fail-decisions")) throw new Error("Test: connection lost");
        const rows = command === "respond_permission" ? pending.permissions : pending.userInputs;
        const index = rows.findIndex((r) => r.requestId === args.requestId);
        if (index >= 0) rows.splice(index, 1);
        emit("permission_update", { status: "resolved" });
        return { ok: true };
      }
      case "update_app_session": {
        const s = find(args.id);
        if (s) Object.assign(s, args.update);
        return;
      }
      case "cancel_turn": {
        timers.get(args.sessionId)?.forEach(window.clearTimeout);
        timers.delete(args.sessionId);
        const s = find(args.sessionId);
        if (s) s.running = false;
        emit("turn/finished", { appSessionId: args.sessionId, turnId: "cancelled", ok: true, cancelled: true, messages: [] });
        return true;
      }
      case "gateway_list_dirs":
        return { path: "/Users/dev/GitHub", parent: "/Users/dev", entries: [{ name: "jockey", path: "/Users/dev/GitHub/jockey", isGitRepo: true, projectId: "p1" }, { name: "blog", path: "/Users/dev/GitHub/blog", isGitRepo: true, projectId: null }] };
      case "list_app_skills":
      case "list_available_commands_cmd": return [];
      case "prewarm_role_config_cmd": return { configOptions: [], modes: [] };
      default: return undefined;
    }
  };

  return {
    call: call as HttpTransport["call"],
    async listen<T>(event: string, handler: (ev: { payload: T }) => void): Promise<UnlistenFn> {
      const set = handlers.get(event) ?? new Set<Handler>();
      set.add(handler as Handler);
      handlers.set(event, set);
      return () => set.delete(handler as Handler);
    },
    openExternal: (url) => { window.open(url, "_blank", "noopener,noreferrer"); },
    setFocus: () => {},
    onResync: () => () => {},
    onUnauthorized: () => {},
    setToken: () => {},
    hasToken: () => true,
  };
}
