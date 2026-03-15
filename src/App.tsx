import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";

type Team = { id: string; name: string; workspacePath: string; createdAt: number };
type Role = { id: string; teamId: string; roleName: string; runtimeKind: string; systemPrompt: string };
type AssistantRuntime = { key: string; label: string; binary: string; available: boolean; version: string | null };
type ChatCommandResult = { ok: boolean; message: string; selectedTeamId: string | null; selectedAssistant: string | null; sessionId: string | null; payload: Record<string, unknown> };
type AssistantChatResponse = { ok: boolean; reply: string; selectedTeamId: string | null; selectedAssistant: string | null; sessionId: string | null; commandResult: ChatCommandResult | null };
type SessionUpdateEvent = { sessionId: string; teamId: string; roleName: string; delta: string; done: boolean };
type WorkflowStateEvent = { sessionId: string; teamId: string; status: string; activeRole: string | null; message: string };
type AcpDeltaEvent = { role: string; delta: string };
type ChatMessage = { id: string; role: "system" | "user" | "assistant" | "event"; text: string; at: number };

const now = () => Date.now();
const fmt = (ts: number) => new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

const RUNTIMES = ["gemini-cli", "claude-code", "codex-cli", "mock"];
const RUNTIME_COLOR: Record<string, string> = {
  "gemini-cli": "text-blue-300",
  "claude-code": "text-orange-300",
  "codex-cli": "text-purple-300",
  mock: "text-zinc-400",
};

export default function App() {
  const [teams, setTeams] = createSignal<Team[]>([]);
  const [roles, setRoles] = createSignal<Role[]>([]);
  const [assistants, setAssistants] = createSignal<AssistantRuntime[]>([]);
  const [selectedTeamId, setSelectedTeamId] = createSignal<string | null>(null);
  const [selectedAssistant, setSelectedAssistant] = createSignal<string | null>(null);
  const [systemPrompt, setSystemPrompt] = createSignal("You are UnionAI assistant. Use UnionAI commands to complete CRUD operations for team/role/workflow/session/context.");
  const [input, setInput] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [streamingId, setStreamingId] = createSignal<string | null>(null);
  let streamBuffer = "";
  let streamFlushTimer: ReturnType<typeof setTimeout> | null = null;

  // Team creation
  const [newTeamName, setNewTeamName] = createSignal("");

  // Role creation form
  const [showRoleForm, setShowRoleForm] = createSignal(false);
  const [roleFormName, setRoleFormName] = createSignal("Developer");
  const [roleFormRuntime, setRoleFormRuntime] = createSignal("gemini-cli");
  const [roleFormPrompt, setRoleFormPrompt] = createSignal("You are a senior developer. Implement the solution step by step.");

  const scrollToBottom = () => {
    setTimeout(() => {
      const el = document.getElementById("msg-list");
      if (el) el.scrollTop = el.scrollHeight;
    }, 0);
  };

  const pushMessage = (role: ChatMessage["role"], text: string) => {
    setMessages((prev) => [...prev, { id: `${now()}-${Math.random().toString(36).slice(2)}`, role, text, at: now() }]);
    scrollToBottom();
  };

  // Start a streaming assistant message, return its id
  const startStream = () => {
    const id = `stream-${now()}`;
    setMessages((prev) => [...prev, { id, role: "assistant", text: "", at: now() }]);
    setStreamingId(id);
    return id;
  };

  const appendStream = (id: string, chunk: string) => {
    setMessages((prev) => prev.map((m) => m.id === id ? { ...m, text: m.text + chunk } : m));
    scrollToBottom();
  };

  const flushStreamBuffer = () => {
    const id = streamingId();
    if (id && streamBuffer) {
      appendStream(id, streamBuffer);
      streamBuffer = "";
    }
    if (streamFlushTimer !== null) {
      clearTimeout(streamFlushTimer);
      streamFlushTimer = null;
    }
  };

  const scheduleStreamFlush = () => {
    if (streamFlushTimer !== null) return;
    streamFlushTimer = window.setTimeout(() => {
      flushStreamBuffer();
    }, 15);
  };

  const resetStreamState = () => {
    streamBuffer = "";
    if (streamFlushTimer !== null) {
      clearTimeout(streamFlushTimer);
      streamFlushTimer = null;
    }
    setStreamingId(null);
  };

  const refreshRoles = async (teamId: string) => {
    try {
      const rows = await invoke<Role[]>("list_roles", { teamId });
      setRoles(rows);
    } catch { setRoles([]); }
  };

  const selectTeam = async (teamId: string) => {
    setSelectedTeamId(teamId);
    await refreshRoles(teamId);
  };

  const refreshTeams = async () => {
    const rows = await invoke<Team[]>("list_teams");
    setTeams(rows);
    if (!selectedTeamId() && rows.length > 0) await selectTeam(rows[0].id);
    else if (selectedTeamId()) await refreshRoles(selectedTeamId()!);
  };

  const refreshAssistants = async () => {
    const rows = await invoke<AssistantRuntime[]>("detect_assistants");
    setAssistants(rows);
    if (!selectedAssistant()) {
      const first = rows.find((a) => a.available);
      if (first) setSelectedAssistant(first.key);
    }
  };

  // Commands that mutate team/role structure and require a sidebar refresh
  const MUTATING_CMDS = ["/team", "/role", "/init", "/workflow"];
  const needsRefresh = (text: string) => MUTATING_CMDS.some((c) => text.startsWith(c));

  const sendRaw = async (text: string, silent = false) => {
    if (!silent) pushMessage("user", text);
    setSubmitting(true);

    const isCommand = text.startsWith("/");
    let streamId: string | null = null;
    if (!isCommand && selectedAssistant()) {
      streamId = startStream();
    }

    try {
      const res = await invoke<AssistantChatResponse>("assistant_chat", {
        input: { input: text, selectedTeamId: selectedTeamId(), selectedAssistant: selectedAssistant(), systemPrompt: null }
      });
      if (res.selectedTeamId) setSelectedTeamId(res.selectedTeamId);
      if (res.selectedAssistant) setSelectedAssistant(res.selectedAssistant);

      if (streamId) {
        resetStreamState();
        setMessages((prev) => prev.map((m) => m.id === streamId ? { ...m, text: res.reply } : m));
      } else {
        pushMessage("assistant", res.reply);
      }

      // Only refresh sidebar when team/role structure may have changed
      if (needsRefresh(text)) {
        await refreshTeams();
      } else if (res.selectedTeamId && res.selectedTeamId !== selectedTeamId()) {
        await refreshRoles(res.selectedTeamId);
      }
    } catch (e) {
      if (streamId) {
        resetStreamState();
        setMessages((prev) => prev.filter((m) => m.id !== streamId));
      }
      pushMessage("event", String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const handleSend = async (e: SubmitEvent) => {
    e.preventDefault();
    const text = input().trim();
    if (!text) return;
    if (!selectedAssistant() && !text.startsWith("/")) {
      pushMessage("system", "请先在左侧选择一个 Assistant。");
      return;
    }
    setInput("");
    await sendRaw(text);
  };

  const handleCreateTeam = async () => {
    const name = newTeamName().trim();
    if (!name) return;
    setNewTeamName("");
    await sendRaw(`/team create ${name}`);
  };

  const handleCreateRole = async () => {
    const name = roleFormName().trim();
    const runtime = roleFormRuntime();
    const prompt = roleFormPrompt().trim();
    if (!name) return;
    const cmd = `/role bind ${name} ${runtime} ${prompt || "You are a helpful AI assistant."}`;
    setShowRoleForm(false);
    await sendRaw(cmd);
    if (selectedTeamId()) await refreshRoles(selectedTeamId()!);
  };

  onMount(() => {
    const handlers: UnlistenFn[] = [];
    pushMessage("system", "欢迎使用 UnionAI。Gemini CLI 正在后台初始化会话，首次对话无需等待冷启动。");

    void refreshAssistants();
    void refreshTeams();

    void Promise.all([
      listen<AcpDeltaEvent>("acp/delta", (ev) => {
        let currentId = streamingId();
        if (!currentId) {
          currentId = `stream-${now()}`;
          setMessages((prev) => [...prev, { id: currentId as string, role: "assistant", text: "", at: now() }]);
          setStreamingId(currentId);
        }
        streamBuffer += ev.payload.delta;
        if (streamBuffer.length >= 6) {
          flushStreamBuffer();
          return;
        }
        scheduleStreamFlush();
      }),
      listen<SessionUpdateEvent>("session/update", (ev) => {
        if (ev.payload.delta) pushMessage("event", `[${ev.payload.roleName}] ${ev.payload.delta}`);
      }),
      listen<WorkflowStateEvent>("workflow/state_changed", (ev) => {
        const p = ev.payload;
        pushMessage("event", `[workflow] ${p.status} ${p.activeRole ?? ""} ${p.message}`);
      }),
    ]).then((hs) => handlers.push(...hs));

    onCleanup(() => {
      resetStreamState();
      handlers.forEach((h) => h());
    });
  });

  const selectedTeam = () => teams().find((t) => t.id === selectedTeamId());

  return (
    <div class="min-h-screen bg-[var(--ui-bg)] text-[var(--ui-text)]">
      <div class="mx-auto grid min-h-screen max-w-[1600px] grid-cols-1 gap-3 px-3 py-3 lg:grid-cols-[260px_1fr]">

        {/* ── Sidebar ── */}
        <aside class="card flex h-[calc(100vh-1.5rem)] flex-col overflow-hidden p-0">

          {/* Assistants */}
          <div class="shrink-0 border-b border-[var(--ui-border)] p-3">
            <div class="mb-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--ui-muted)]">Assistant</div>
            <div class="space-y-1">
              <For each={assistants()}>
                {(a) => (
                  <button
                    class={`flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left text-xs transition-colors ${
                      selectedAssistant() === a.key
                        ? "border-[var(--ui-accent)] bg-[var(--ui-accent-soft)]"
                        : "border-transparent bg-[var(--ui-panel-2)] hover:border-[var(--ui-border)]"
                    } ${!a.available ? "opacity-40" : ""}`}
                    onClick={() => a.available && setSelectedAssistant(a.key)}
                  >
                    <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${a.available ? "bg-emerald-400" : "bg-rose-400"}`} />
                    <span class="flex-1 font-medium">{a.label}</span>
                    <Show when={a.version}><span class="text-[10px] text-[var(--ui-muted)]">v{a.version}</span></Show>
                  </button>
                )}
              </For>
            </div>
          </div>

          {/* Teams */}
          <div class="shrink-0 border-b border-[var(--ui-border)] p-3">
            <div class="mb-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--ui-muted)]">Teams</div>
            <div class="mb-2 flex gap-1.5">
              <input
                value={newTeamName()}
                onInput={(e) => setNewTeamName(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && void handleCreateTeam()}
                placeholder="team name…"
                class="h-7 min-w-0 flex-1 rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 text-xs outline-none placeholder:text-[var(--ui-muted)]"
              />
              <button onClick={() => void handleCreateTeam()} class="h-7 rounded bg-[var(--ui-accent)] px-2.5 text-xs font-bold text-[var(--ui-accent-text)]">+</button>
            </div>
            <div class="space-y-1 max-h-32 overflow-auto">
              <For each={teams()}>
                {(t) => (
                  <button
                    class={`flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left text-xs transition-colors ${
                      selectedTeamId() === t.id
                        ? "border-[var(--ui-accent)] bg-[var(--ui-accent-soft)]"
                        : "border-transparent bg-[var(--ui-panel-2)] hover:border-[var(--ui-border)]"
                    }`}
                    onClick={() => void selectTeam(t.id)}
                  >
                    <span class="flex-1 font-medium">{t.name}</span>
                    <span class="text-[9px] text-[var(--ui-muted)]">{t.id.slice(0, 6)}…</span>
                  </button>
                )}
              </For>
              <Show when={teams().length === 0}>
                <p class="rounded-lg border border-dashed border-[var(--ui-border)] p-2 text-center text-[10px] text-[var(--ui-muted)]">暂无 team</p>
              </Show>
            </div>
          </div>

          {/* Roles */}
          <div class="flex flex-1 flex-col overflow-hidden p-3">
            <div class="mb-2 flex items-center justify-between">
              <span class="text-[10px] font-semibold uppercase tracking-wider text-[var(--ui-muted)]">
                Roles{selectedTeam() ? ` · ${selectedTeam()!.name}` : ""}
              </span>
              <button
                onClick={() => setShowRoleForm((v) => !v)}
                class="rounded border border-[var(--ui-border)] px-1.5 py-0.5 text-[10px] text-[var(--ui-muted)] hover:text-[var(--ui-text)]"
              >
                {showRoleForm() ? "cancel" : "+ role"}
              </button>
            </div>

            {/* Role creation form */}
            <Show when={showRoleForm()}>
              <div class="mb-2 space-y-1.5 rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel-2)] p-2">
                <input
                  value={roleFormName()}
                  onInput={(e) => setRoleFormName(e.currentTarget.value)}
                  placeholder="Role name (e.g. Developer)"
                  class="h-7 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 text-xs outline-none"
                />
                <select
                  value={roleFormRuntime()}
                  onChange={(e) => setRoleFormRuntime(e.currentTarget.value)}
                  class="h-7 w-full rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 text-xs outline-none"
                >
                  <For each={RUNTIMES}>{(r) => <option value={r}>{r}</option>}</For>
                </select>
                <textarea
                  value={roleFormPrompt()}
                  onInput={(e) => setRoleFormPrompt(e.currentTarget.value)}
                  rows={3}
                  placeholder="System prompt for this role…"
                  class="w-full resize-none rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 py-1 text-xs outline-none"
                />
                <button
                  onClick={() => void handleCreateRole()}
                  class="h-7 w-full rounded bg-[var(--ui-accent)] text-xs font-semibold text-[var(--ui-accent-text)]"
                >
                  Create Role
                </button>
              </div>
            </Show>

            {/* Roles list */}
            <div class="flex-1 space-y-1 overflow-auto">
              <For each={roles()}>
                {(role) => (
                  <div class="group flex items-start gap-1.5 rounded-lg bg-[var(--ui-panel-2)] px-2 py-1.5">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-1.5">
                        <span class="text-xs font-medium">{role.roleName}</span>
                        <span class={`text-[10px] ${RUNTIME_COLOR[role.runtimeKind] ?? "text-[var(--ui-muted)]"}`}>{role.runtimeKind}</span>
                      </div>
                      <p class="truncate text-[10px] text-[var(--ui-muted)]">{role.systemPrompt}</p>
                    </div>
                    <button
                      onClick={() => void sendRaw(`@${role.roleName} hello`)}
                      class="shrink-0 rounded px-1 py-0.5 text-[9px] text-[var(--ui-muted)] opacity-0 hover:text-[var(--ui-text)] group-hover:opacity-100"
                    >
                      chat
                    </button>
                  </div>
                )}
              </For>
              <Show when={roles().length === 0 && !showRoleForm()}>
                <p class="rounded-lg border border-dashed border-[var(--ui-border)] p-2 text-center text-[10px] text-[var(--ui-muted)]">
                  暂无 role，点击 "+ role" 添加
                </p>
              </Show>
            </div>

            {/* Quick actions */}
            <div class="mt-2 flex flex-wrap gap-1 border-t border-[var(--ui-border)] pt-2">
              <button onClick={() => void sendRaw("/workflow list")} class="rounded border border-[var(--ui-border)] px-1.5 py-0.5 text-[10px] text-[var(--ui-muted)] hover:text-[var(--ui-text)]">workflows</button>
              <button onClick={() => void sendRaw("/context list")} class="rounded border border-[var(--ui-border)] px-1.5 py-0.5 text-[10px] text-[var(--ui-muted)] hover:text-[var(--ui-text)]">context</button>
              <button onClick={() => void sendRaw("/session list")} class="rounded border border-[var(--ui-border)] px-1.5 py-0.5 text-[10px] text-[var(--ui-muted)] hover:text-[var(--ui-text)]">sessions</button>
            </div>
          </div>
        </aside>

        {/* ── Chat ── */}
        <main class="card flex h-[calc(100vh-1.5rem)] flex-col overflow-hidden p-0">

          {/* Header */}
          <div class="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--ui-border)] px-4 py-2">
            <div>
              <h1 class="text-sm font-semibold">UnionAI Chat</h1>
              <p class="text-[11px] text-[var(--ui-muted)]">
                <Show when={selectedTeam()} fallback={<span>no team</span>}>{selectedTeam()!.name}</Show>
                {" · "}
                <span class={selectedAssistant() ? RUNTIME_COLOR[selectedAssistant()!] ?? "text-emerald-400" : "text-rose-400"}>
                  {selectedAssistant() ?? "no assistant"}
                </span>
              </p>
            </div>
            <button onClick={() => { void refreshAssistants(); void refreshTeams(); }} class="rounded border border-[var(--ui-border)] bg-[var(--ui-panel-2)] px-2.5 py-1 text-xs hover:bg-[var(--ui-panel)]">
              Refresh
            </button>
          </div>

          {/* System prompt */}
          <div class="shrink-0 border-b border-[var(--ui-border)] bg-[var(--ui-panel-2)] px-4 py-2">
            <textarea
              value={systemPrompt()}
              onInput={(e) => setSystemPrompt(e.currentTarget.value)}
              rows={2}
              class="w-full resize-none rounded border border-[var(--ui-border)] bg-[var(--ui-panel)] px-2 py-1 text-xs outline-none"
              placeholder="System prompt…"
            />
          </div>

          {/* Messages */}
          <div id="msg-list" class="flex-1 overflow-auto p-3 space-y-1.5">
            <For each={messages()}>
              {(msg) => (
                <div class={`rounded-lg border px-3 py-2 ${
                  msg.role === "user"
                    ? "ml-10 border-[var(--ui-accent)] bg-[var(--ui-accent-soft)]"
                    : msg.role === "assistant"
                    ? "mr-10 border-[var(--ui-border)] bg-[var(--ui-panel-2)]"
                    : "border-[var(--ui-border)] bg-black/10 opacity-70"
                }`}>
                  <div class="mb-0.5 flex items-center justify-between text-[10px] text-[var(--ui-muted)]">
                    <span class="font-medium">{msg.role}</span>
                    <span>{fmt(msg.at)}</span>
                  </div>
                  <pre class="whitespace-pre-wrap break-words text-xs leading-relaxed">{msg.text}</pre>
                </div>
              )}
            </For>
            <Show when={submitting()}>
              <div class="flex items-center gap-2 px-1 text-xs text-[var(--ui-muted)]">
                <span class="animate-pulse">●</span> 等待响应…
              </div>
            </Show>
          </div>

          {/* Input */}
          <form onSubmit={handleSend} class="shrink-0 border-t border-[var(--ui-border)] p-3 flex gap-2">
            <input
              value={input()}
              onInput={(e) => setInput(e.currentTarget.value)}
              placeholder="自然语言 / /命令 / @Role 路由"
              disabled={submitting()}
              class="h-9 flex-1 rounded-lg border border-[var(--ui-border)] bg-[var(--ui-panel-2)] px-3 text-sm outline-none disabled:opacity-50"
            />
            <button type="submit" disabled={submitting()} class="h-9 rounded-lg bg-[var(--ui-accent)] px-4 text-sm font-semibold text-[var(--ui-accent-text)] disabled:opacity-50">
              {submitting() ? "…" : "Send"}
            </button>
          </form>
        </main>

      </div>
    </div>
  );
}
