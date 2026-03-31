import { For, Show, createMemo, createSignal } from "solid-js";
import type { Role } from "../types";
import { Badge, EmptyState, FieldRow, TextInput, InlineSelect, ActionButton, PanelSection } from "./primitives";
import type { AcpMcpServer } from "./primitives";
import { mcpTransport, parseCommandArgs } from "./primitives";
import { roleApi } from "../../lib/tauriApi";

type McpEntry = { key: string; server: AcpMcpServer; roleName: string; index: number };

function parseRoleMcpServers(role: Role): McpEntry[] {
  try {
    const arr = JSON.parse(role.mcpServersJson || "[]") as AcpMcpServer[];
    return arr.map((s, i) => ({ key: `${role.id}-${i}`, server: s, roleName: role.roleName, index: i }));
  } catch { return []; }
}

export function McpRegistryTab(props: { roles: Role[]; refreshRoles: () => Promise<void>; pushMessage: (role: string, text: string) => void }) {
  const allEntries = createMemo(() => props.roles.flatMap(parseRoleMcpServers));
  const [selectedKey, setSelectedKey] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  const [fName, setFName] = createSignal("");
  const [fTransport, setFTransport] = createSignal<"stdio" | "http" | "sse">("stdio");
  const [fCommand, setFCommand] = createSignal("");
  const [fArgs, setFArgs] = createSignal("");
  const [fUrl, setFUrl] = createSignal("");
  const [fEnvText, setFEnvText] = createSignal("");
  const [fHeadersText, setFHeadersText] = createSignal("");
  const [fRole, setFRole] = createSignal("");
  const parsedArgs = createMemo(() => parseCommandArgs(fArgs().trim()));
  const argsInvalid = createMemo(() => fTransport() === "stdio" && !!fArgs().trim() && parsedArgs() === null);

  const selected = createMemo(() => allEntries().find((e) => e.key === selectedKey()) ?? null);

  const transportBadge = (t: string) => ({
    stdio: "bg-amber-500/15 text-amber-300",
    http: "bg-sky-500/15 text-sky-300",
    sse: "bg-violet-500/15 text-violet-300",
  }[t] ?? "bg-[var(--ui-surface-muted)] theme-muted");

  const roleOptions = createMemo(() =>
    props.roles.filter((r) => r.roleName !== "UnionAIAssistant").map((r) => ({ value: r.roleName, label: r.roleName }))
  );

  function parseEnvPairs(text: string): Array<{ name: string; value: string }> {
    return text.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
      const eq = l.indexOf("=");
      if (eq < 0) return { name: l, value: "" };
      return { name: l.slice(0, eq), value: l.slice(eq + 1) };
    });
  }

  function parseHeaderPairs(text: string): Array<{ name: string; value: string }> {
    return parseEnvPairs(text);
  }

  function isSensitiveKey(name: string): boolean {
    const n = name.trim().toLowerCase();
    return n.includes("authorization") || n.includes("api-key") || n.includes("token") || n.includes("secret") || n.includes("cookie");
  }

  function redactServerForDisplay(server: AcpMcpServer): AcpMcpServer {
    if ("headers" in server) {
      return {
        ...server,
        headers: server.headers.map((h) => ({ name: h.name, value: isSensitiveKey(h.name) ? "********" : h.value })),
      };
    }
    return server;
  }

  function buildServer(): AcpMcpServer | null {
    const name = fName().trim();
    if (!name) return null;
    const t = fTransport();
    if (t === "stdio") {
      const command = fCommand().trim();
      if (!command) return null;
      const args = fArgs().trim() ? parsedArgs() : [];
      if (args === null) return null;
      return { name, command, args, env: parseEnvPairs(fEnvText()) };
    }
    const url = fUrl().trim();
    if (!url) return null;
    return { type: t, name, url, headers: parseHeaderPairs(fHeadersText()) };
  }

  async function handleAdd() {
    const server = buildServer();
    const roleName = fRole().trim();
    if (!server || !roleName) return;
    const role = props.roles.find((r) => r.roleName === roleName);
    if (!role) return;
    setSaving(true);
    try {
      const existing: AcpMcpServer[] = JSON.parse(role.mcpServersJson || "[]");
      existing.push(server);
      await roleApi.upsert({
        roleName: role.roleName, runtimeKind: role.runtimeKind,
        systemPrompt: role.systemPrompt, model: role.model ?? null,
        mode: role.mode ?? null, mcpServersJson: JSON.stringify(existing),
        configOptionsJson: role.configOptionsJson, autoApprove: role.autoApprove,
      });
      await props.refreshRoles();
      setCreating(false);
      setFName(""); setFCommand(""); setFArgs(""); setFUrl(""); setFEnvText(""); setFHeadersText("");
      setFRole("");
    } catch (e) { props.pushMessage("event", `Failed to add MCP server: ${String(e)}`); }
    finally { setSaving(false); }
  }

  async function handleRemove(entry: McpEntry) {
    try {
      const role = props.roles.find((r) => r.roleName === entry.roleName);
      if (!role) return;
      const existing: AcpMcpServer[] = JSON.parse(role.mcpServersJson || "[]");
      if (entry.index < 0 || entry.index >= existing.length) return;
      existing.splice(entry.index, 1);
      await roleApi.upsert({
        roleName: role.roleName, runtimeKind: role.runtimeKind,
        systemPrompt: role.systemPrompt, model: role.model ?? null,
        mode: role.mode ?? null, mcpServersJson: JSON.stringify(existing),
        configOptionsJson: role.configOptionsJson, autoApprove: role.autoApprove,
      });
      if (selectedKey() === entry.key) setSelectedKey(null);
      await props.refreshRoles();
    } catch (e) {
      props.pushMessage("event", `Failed to remove MCP server: ${String(e)}`);
    }
  }

  return (
    <div class="flex h-full">
      <div class="flex w-60 shrink-0 flex-col border-r theme-border">
        <div class="p-3">
          <ActionButton
            label="+ Add MCP Server"
            variant="ghost"
            class="w-full"
            onClick={() => { setCreating(true); setSelectedKey(null); }}
          />
        </div>
        <div class="flex-1 overflow-y-auto space-y-0.5 py-1">
          <Show when={allEntries().length === 0}>
            <EmptyState icon="◈" title="No MCP servers" sub="Add an MCP server to a role" />
          </Show>
          <For each={allEntries()}>
            {(entry) => (
              <button
                onClick={() => { setSelectedKey(entry.key); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 rounded-lg mx-1.5 px-2.5 py-2 text-left transition-colors duration-100 ${selectedKey() === entry.key ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"}`}
              >
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400" />
                  <span class="truncate font-mono text-[10px] font-semibold theme-text">{entry.server.name}</span>
                </div>
                <div class="flex items-center gap-1.5 pl-3">
                  <Badge label={mcpTransport(entry.server)} color={transportBadge(mcpTransport(entry.server))} />
                  <span class="font-mono text-[9px] theme-muted truncate">{entry.roleName}</span>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>

      <div class="flex-1 overflow-y-auto p-5">
        <Show when={creating()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold theme-text uppercase tracking-widest">Add MCP Server</h3>
            <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
              <FieldRow label="Name">
                <TextInput value={fName()} onInput={setFName} placeholder="e.g. chrome-devtools" monospace />
              </FieldRow>
              <FieldRow label="Transport">
                <InlineSelect
                  value={fTransport()}
                  options={[
                    { value: "stdio", label: "stdio — subprocess" },
                    { value: "http", label: "http — HTTP streamable" },
                    { value: "sse", label: "sse — Server-Sent Events" },
                  ]}
                  onChange={(v) => setFTransport(v as "stdio" | "http" | "sse")}
                />
              </FieldRow>
              <Show when={fTransport() === "stdio"}>
                <FieldRow label="Command">
                  <TextInput value={fCommand()} onInput={setFCommand} placeholder="npx" monospace />
                </FieldRow>
                <FieldRow label="Args">
                  <TextInput value={fArgs()} onInput={setFArgs} placeholder="-y @anthropic-ai/chrome-devtools-mcp@latest" monospace />
                </FieldRow>
                <Show when={argsInvalid()}>
                  <div class="ml-[92px] text-[10px] text-rose-400 font-mono">Invalid args: check quotes/escaping.</div>
                </Show>
                <FieldRow label="Env">
                  <TextInput value={fEnvText()} onInput={setFEnvText} placeholder="KEY=value (one per line)" multiline rows={2} monospace />
                </FieldRow>
              </Show>
              <Show when={fTransport() !== "stdio"}>
                <FieldRow label="URL">
                  <TextInput value={fUrl()} onInput={setFUrl} placeholder="https://mcp.example.com" monospace />
                </FieldRow>
                <FieldRow label="Headers">
                  <TextInput value={fHeadersText()} onInput={setFHeadersText} placeholder="Authorization=Bearer xxx (one per line)" multiline rows={2} monospace />
                </FieldRow>
              </Show>
              <FieldRow label="Role">
                <InlineSelect value={fRole()} options={roleOptions()} onChange={setFRole} />
              </FieldRow>
            </div>
            <div class="flex gap-2">
              <ActionButton label={saving() ? "Adding…" : "Add"} variant="primary" disabled={saving() || !fName().trim() || !fRole() || argsInvalid()} onClick={() => void handleAdd()} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>

            <Show when={fTransport() === "stdio" && fName().trim() && fCommand().trim()}>
              <PanelSection title="Preview (ACP JSON)">
                <pre class="rounded-lg border theme-border bg-[var(--ui-surface)] p-3 font-mono text-[10px] theme-muted overflow-x-auto whitespace-pre-wrap">
                  {JSON.stringify(buildServer(), null, 2)}
                </pre>
              </PanelSection>
            </Show>
          </div>
        </Show>

        <Show when={!creating() && selected()}>
          {(entry) => {
            const s = () => entry().server;
            const t = () => mcpTransport(s());
            return (
              <div class="space-y-5">
                <div class="flex items-start justify-between gap-4">
                  <div class="flex items-center gap-2">
                    <span class="h-2 w-2 rounded-full bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.4)]" />
                    <h2 class="font-mono text-sm font-bold theme-text">{s().name}</h2>
                    <Badge label={t()} color={transportBadge(t())} />
                  </div>
                  <ActionButton label="Remove" variant="danger" onClick={() => void handleRemove(entry())} />
                </div>

                <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
                  <Show when={t() === "stdio" && "command" in s()}>
                    <FieldRow label="Command">
                      <span class="break-all font-mono text-[10px] theme-text">{(s() as any).command}</span>
                    </FieldRow>
                    <FieldRow label="Args">
                      <span class="break-all font-mono text-[10px] theme-text">{((s() as any).args ?? []).join(" ")}</span>
                    </FieldRow>
                    <Show when={((s() as any).env ?? []).length > 0}>
                      <FieldRow label="Env">
                        <div class="flex flex-col gap-0.5">
                          <For each={(s() as any).env}>
                            {(e: { name: string; value: string }) => (
                              <span class="font-mono text-[10px] theme-muted">{e.name}={e.value}</span>
                            )}
                          </For>
                        </div>
                      </FieldRow>
                    </Show>
                  </Show>
                  <Show when={t() !== "stdio" && "url" in s()}>
                    <FieldRow label="URL">
                      <span class="break-all font-mono text-[10px] theme-text">{(s() as any).url}</span>
                    </FieldRow>
                    <Show when={((s() as any).headers ?? []).length > 0}>
                      <FieldRow label="Headers">
                        <div class="flex flex-col gap-0.5">
                            <For each={(s() as any).headers}>
                              {(h: { name: string; value: string }) => (
                                <span class="font-mono text-[10px] theme-muted">{h.name}: {isSensitiveKey(h.name) ? "********" : h.value}</span>
                              )}
                            </For>
                          </div>
                      </FieldRow>
                    </Show>
                  </Show>
                  <FieldRow label="Transport">
                    <Badge label={t()} color={transportBadge(t())} />
                  </FieldRow>
                  <FieldRow label="Role">
                    <span class="font-mono text-[10px] theme-muted">{entry().roleName}</span>
                  </FieldRow>
                </div>

                <PanelSection title="ACP JSON">
                  <pre class="rounded-lg border theme-border bg-[var(--ui-surface)] p-3 font-mono text-[10px] theme-muted overflow-x-auto whitespace-pre-wrap">
                    {JSON.stringify(redactServerForDisplay(s()), null, 2)}
                  </pre>
                </PanelSection>
              </div>
            );
          }}
        </Show>

        <Show when={!creating() && !selected()}>
          <EmptyState icon="◈" title="Select an MCP server" sub="Or add a new one to a role" />
        </Show>
      </div>
    </div>
  );
}
