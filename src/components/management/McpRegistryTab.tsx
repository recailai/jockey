import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { Badge, EmptyState, FieldRow, TextInput, InlineSelect, ActionButton, PanelSection } from "./primitives";
import type { AcpMcpServer } from "./primitives";
import { mcpTransport, parseCommandArgs } from "./primitives";
import { globalMcpApi, type GlobalMcpEntry } from "../../lib/tauriApi";

type McpEntry = { key: string; server: AcpMcpServer; isBuiltin: boolean };

function parseEntry(entry: GlobalMcpEntry): McpEntry | null {
  try {
    const server = JSON.parse(entry.configJson) as AcpMcpServer;
    return { key: entry.name, server, isBuiltin: entry.isBuiltin };
  } catch { return null; }
}

export function McpRegistryTab(props: { pushMessage: (role: string, text: string) => void }) {
  const [entries, setEntries] = createSignal<McpEntry[]>([]);
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
  const parsedArgs = createMemo(() => parseCommandArgs(fArgs().trim()));
  const argsInvalid = createMemo(() => fTransport() === "stdio" && !!fArgs().trim() && parsedArgs() === null);

  const selected = createMemo(() => entries().find((e) => e.key === selectedKey()) ?? null);

  async function refresh() {
    try {
      const raw = await globalMcpApi.list();
      setEntries(raw.map(parseEntry).filter((e): e is McpEntry => e !== null));
    } catch (e) {
      props.pushMessage("event", `Failed to load MCP registry: ${String(e)}`);
    }
  }

  onMount(() => { void refresh(); });

  const transportBadge = (t: string) => ({
    stdio: "management-badge-warning",
    http: "management-badge-info",
    sse: "management-badge-muted",
  }[t] ?? "management-badge-muted");

  function parseEnvPairs(text: string): Array<{ name: string; value: string }> {
    return text.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
      const eq = l.indexOf("=");
      if (eq < 0) return { name: l, value: "" };
      return { name: l.slice(0, eq), value: l.slice(eq + 1) };
    });
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
    return { type: t, name, url, headers: parseEnvPairs(fHeadersText()) };
  }

  async function handleAdd() {
    const server = buildServer();
    if (!server) return;
    const name = fName().trim();
    setSaving(true);
    try {
      await globalMcpApi.upsert(name, JSON.stringify(server));
      await refresh();
      setCreating(false);
      setSelectedKey(name);
      setFName(""); setFCommand(""); setFArgs(""); setFUrl(""); setFEnvText(""); setFHeadersText("");
    } catch (e) { props.pushMessage("event", `Failed to add MCP server: ${String(e)}`); }
    finally { setSaving(false); }
  }

  async function handleRemove(entry: McpEntry) {
    if (entry.isBuiltin) return;
    try {
      await globalMcpApi.remove(entry.key);
      if (selectedKey() === entry.key) setSelectedKey(null);
      await refresh();
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
          <Show when={entries().length === 0}>
            <EmptyState icon="◈" title="No MCP servers" sub="Add an MCP server to the registry" />
          </Show>
          <For each={entries()}>
            {(entry) => (
              <button
                onClick={() => { setSelectedKey(entry.key); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 rounded-lg mx-1.5 px-2.5 py-2 text-left transition-colors duration-100 ${selectedKey() === entry.key ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"}`}
              >
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400" />
                  <span class="truncate font-mono text-[10px] font-semibold theme-text">{entry.server.name}</span>
                  <Show when={entry.isBuiltin}>
                    <Badge label="built-in" color="management-badge-muted" />
                  </Show>
                </div>
                <div class="flex items-center gap-1.5 pl-3">
                  <Badge label={mcpTransport(entry.server)} color={transportBadge(mcpTransport(entry.server))} />
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
            </div>
            <div class="flex gap-2">
              <ActionButton label={saving() ? "Adding…" : "Add"} variant="primary" disabled={saving() || !fName().trim() || argsInvalid()} onClick={() => void handleAdd()} />
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
                    <Show when={entry().isBuiltin}>
                      <Badge label="built-in" color="management-badge-muted" />
                    </Show>
                  </div>
                  <Show when={!entry().isBuiltin}>
                    <ActionButton label="Remove" variant="danger" onClick={() => void handleRemove(entry())} />
                  </Show>
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
          <EmptyState icon="◈" title="Select an MCP server" sub="Or add a new one to the registry" />
        </Show>
      </div>
    </div>
  );
}
