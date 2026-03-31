import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import type { Role } from "../types";
import { Badge, EmptyState, FieldRow, TextInput, InlineSelect, ActionButton, PanelSection } from "./primitives";
import type { McpServer } from "./primitives";

export function McpRegistryTab(props: { roles: Role[] }) {
  const [servers, setServers] = createSignal<McpServer[]>([]);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);

  // Form state
  const [fName, setFName] = createSignal("");
  const [fUri, setFUri] = createSignal("");
  const [fTransport, setFTransport] = createSignal<"stdio" | "http" | "sse">("stdio");
  const [fRole, setFRole] = createSignal("");

  // Seed from existing roles' mcpServersJson on mount
  onMount(() => {
    const discovered: McpServer[] = [];
    props.roles.forEach((role) => {
      try {
        const arr = JSON.parse(role.mcpServersJson || "[]") as Array<{ name?: string; uri?: string; url?: string; transport?: string }>;
        arr.forEach((entry, idx) => {
          discovered.push({
            id: `${role.id}-mcp-${idx}`,
            name: entry.name ?? `${role.roleName}#${idx}`,
            uri: entry.uri ?? entry.url ?? "",
            transport: (entry.transport as "stdio" | "http" | "sse") ?? "stdio",
            enabled: true,
            capabilities: [],
            roleName: role.roleName,
          });
        });
      } catch { /* ignore */ }
    });
    setServers(discovered);
    if (discovered.length > 0) setSelectedId(discovered[0].id);
  });

  const selected = createMemo(() => servers().find((s) => s.id === selectedId()) ?? null);

  const transportBadge = (t: string) => ({
    stdio: "bg-amber-500/15 text-amber-300",
    http: "bg-sky-500/15 text-sky-300",
    sse: "bg-violet-500/15 text-violet-300",
  }[t] ?? "bg-zinc-800 text-zinc-400");

  const handleAdd = () => {
    const name = fName().trim();
    if (!name || !fUri().trim()) return;
    const newServer: McpServer = {
      id: `local-${Date.now()}`,
      name,
      uri: fUri().trim(),
      transport: fTransport(),
      enabled: true,
      capabilities: [],
      roleName: fRole() || undefined,
    };
    setServers((s) => [...s, newServer]);
    setSelectedId(newServer.id);
    setCreating(false);
    setFName(""); setFUri(""); setFTransport("stdio"); setFRole("");
  };

  const toggleEnabled = (id: string) =>
    setServers((s) => s.map((srv) => srv.id === id ? { ...srv, enabled: !srv.enabled } : srv));

  const removeServer = (id: string) => {
    setServers((s) => s.filter((srv) => srv.id !== id));
    if (selectedId() === id) setSelectedId(null);
  };

  const roleOptions = createMemo(() =>
    [{ value: "", label: "— Global (all roles) —" }, ...props.roles.map((r) => ({ value: r.roleName, label: r.roleName }))]
  );

  return (
    <div class="flex h-full">
      {/* List */}
      <div class="flex w-60 shrink-0 flex-col border-r border-white/[0.04]">
        <div class="border-b border-white/[0.04] p-3">
          <ActionButton
            label="+ Register Server"
            variant="ghost"
            class="w-full"
            onClick={() => { setCreating(true); setSelectedId(null); }}
          />
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={servers().length === 0}>
            <EmptyState icon="◈" title="No MCP servers" sub="Servers are imported from role configurations" />
          </Show>
          <For each={servers()}>
            {(srv) => (
              <button
                onClick={() => { setSelectedId(srv.id); setCreating(false); }}
                class={`group flex w-full flex-col gap-0.5 border-b border-white/[0.03] px-3 py-2.5 text-left transition-colors duration-100 ${selectedId() === srv.id ? "bg-[var(--ui-surface-muted)]" : "hover:bg-[var(--ui-surface-muted)]"} ${!srv.enabled ? "opacity-40" : ""}`}
              >
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${srv.enabled ? "bg-emerald-400" : "bg-zinc-700"}`} />
                  <span class={`truncate font-mono text-[10px] font-semibold ${selectedId() === srv.id ? "text-zinc-100" : "text-zinc-300"}`}>{srv.name}</span>
                </div>
                <div class="flex items-center gap-1.5 pl-3">
                  <Badge label={srv.transport} color={transportBadge(srv.transport)} />
                  <Show when={srv.roleName}>
                    <span class="font-mono text-[9px] text-zinc-600 truncate">{srv.roleName}</span>
                  </Show>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>

      {/* Detail */}
      <div class="flex-1 overflow-y-auto p-5">
        <Show when={creating()}>
          <div class="space-y-4">
            <h3 class="font-mono text-xs font-bold text-zinc-300 uppercase tracking-widest">Register MCP Server</h3>
            <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
              <FieldRow label="Name">
                <TextInput value={fName()} onInput={setFName} placeholder="e.g. filesystem" />
              </FieldRow>
              <FieldRow label="URI">
                <TextInput value={fUri()} onInput={setFUri} placeholder="npx @modelcontextprotocol/server-filesystem" monospace />
              </FieldRow>
              <FieldRow label="Transport">
                <InlineSelect
                  value={fTransport()}
                  options={[
                    { value: "stdio", label: "stdio — subprocess" },
                    { value: "http", label: "http — HTTP/JSON-RPC" },
                    { value: "sse", label: "sse — Server-Sent Events" },
                  ]}
                  onChange={(v) => setFTransport(v as "stdio" | "http" | "sse")}
                />
              </FieldRow>
              <FieldRow label="Role">
                <InlineSelect value={fRole()} options={roleOptions()} onChange={setFRole} />
              </FieldRow>
            </div>
            <div class="flex gap-2">
              <ActionButton label="Add" variant="primary" onClick={handleAdd} />
              <ActionButton label="Cancel" variant="ghost" onClick={() => setCreating(false)} />
            </div>
          </div>
        </Show>

        <Show when={!creating() && selected()}>
          {(srv) => (
            <div class="space-y-5">
              <div class="flex items-start justify-between gap-4">
                <div class="flex items-center gap-2">
                  <span class={`h-2 w-2 rounded-full ${srv().enabled ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.4)]" : "bg-zinc-700"}`} />
                  <h2 class="font-mono text-sm font-bold text-zinc-100">{srv().name}</h2>
                  <Badge label={srv().transport} color={transportBadge(srv().transport)} />
                </div>
                <div class="flex gap-2 shrink-0">
                  <ActionButton
                    label={srv().enabled ? "Disable" : "Enable"}
                    variant="ghost"
                    onClick={() => toggleEnabled(srv().id)}
                  />
                  <ActionButton label="Remove" variant="danger" onClick={() => removeServer(srv().id)} />
                </div>
              </div>

              <div class="space-y-2 rounded-lg border theme-border bg-[var(--ui-surface-muted)] p-4">
                <FieldRow label="URI">
                  <span class="break-all font-mono text-[10px] text-zinc-300">{srv().uri}</span>
                </FieldRow>
                <FieldRow label="Transport">
                  <Badge label={srv().transport} color={transportBadge(srv().transport)} />
                </FieldRow>
                <FieldRow label="Scope">
                  <span class="font-mono text-[10px] text-zinc-400">{srv().roleName ?? "global"}</span>
                </FieldRow>
                <FieldRow label="Status">
                  <div class="flex items-center gap-1.5">
                    <span class={`h-1.5 w-1.5 rounded-full ${srv().enabled ? "bg-emerald-400" : "bg-zinc-700"}`} />
                    <span class="font-mono text-[10px] text-zinc-400">{srv().enabled ? "enabled" : "disabled"}</span>
                  </div>
                </FieldRow>
              </div>

              <Show when={(srv().capabilities?.length ?? 0) > 0}>
                <PanelSection title="Capabilities">
                  <div class="flex flex-wrap gap-1.5">
                    <For each={srv().capabilities}>
                      {(cap) => <Badge label={cap} color="bg-teal-500/15 text-teal-300" />}
                    </For>
                  </div>
                </PanelSection>
              </Show>

              <div class="rounded-lg border border-amber-500/10 bg-amber-500/5 p-3">
                <p class="font-mono text-[10px] text-amber-600/80 leading-relaxed">
                  MCP server configuration is stored in the role's <code class="text-amber-400">mcpServersJson</code> field.
                  Switch to the Roles tab to edit it directly.
                </p>
              </div>
            </div>
          )}
        </Show>

        <Show when={!creating() && !selected()}>
          <EmptyState icon="◈" title="Select an MCP server" sub="Or register a new one" />
        </Show>
      </div>
    </div>
  );
}
