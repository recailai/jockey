import { Show, createMemo, createSignal } from "solid-js";
import { Portal } from "solid-js/web";
import { INTERACTIVE_MOTION } from "./types";
import { FieldRow, TextInput, InlineSelect } from "./management/primitives";
import type { AcpMcpServer } from "./management/primitives";
import { parseCommandArgs } from "./management/primitives";
import { globalMcpApi } from "../lib/tauriApi";

type Props = {
  open: () => boolean;
  onClose: () => void;
  onAdded?: (name: string) => void;
  defaultRoleName?: string;
};

function parseEnvPairs(text: string): Array<{ name: string; value: string }> {
  return text.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
    const eq = l.indexOf("=");
    if (eq < 0) return { name: l, value: "" };
    return { name: l.slice(0, eq), value: l.slice(eq + 1) };
  });
}

function buildServer(
  name: string,
  transport: "stdio" | "http" | "sse",
  command: string,
  args: string,
  url: string,
  envText: string,
  headersText: string,
): AcpMcpServer | null {
  if (!name.trim()) return null;
  const parsedArgs = args.trim() ? parseCommandArgs(args.trim()) : [];
  if (transport === "stdio") {
    if (!command.trim()) return null;
    if (parsedArgs === null) return null;
    return { name: name.trim(), command: command.trim(), args: parsedArgs, env: parseEnvPairs(envText) };
  }
  if (!url.trim()) return null;
  return { type: transport, name: name.trim(), url: url.trim(), headers: parseEnvPairs(headersText) };
}

export default function QuickAddMcpModal(props: Props) {
  const [transport, setTransport] = createSignal<"stdio" | "http" | "sse">("stdio");
  const [name, setName] = createSignal("");
  const [command, setCommand] = createSignal("");
  const [args, setArgs] = createSignal("");
  const [url, setUrl] = createSignal("");
  const [envText, setEnvText] = createSignal("");
  const [headersText, setHeadersText] = createSignal("");
  const [enableForRole, setEnableForRole] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal("");

  const argsInvalid = createMemo(() =>
    transport() === "stdio" && !!args().trim() && parseCommandArgs(args().trim()) === null
  );

  const server = createMemo(() =>
    buildServer(name(), transport(), command(), args(), url(), envText(), headersText())
  );

  const reset = () => {
    setName(""); setCommand(""); setArgs(""); setUrl("");
    setEnvText(""); setHeadersText(""); setError(""); setSaving(false);
    setTransport("stdio"); setEnableForRole(true);
  };

  const handleClose = () => { reset(); props.onClose(); };

  const handleAdd = async () => {
    const srv = server();
    if (!srv || saving()) return;
    setSaving(true);
    setError("");
    try {
      await globalMcpApi.upsert(name().trim(), JSON.stringify(srv));
      if (props.defaultRoleName && enableForRole()) {
        await globalMcpApi.setRoleMcpEnabled(props.defaultRoleName, name().trim(), true);
      }
      props.onAdded?.(name().trim());
      handleClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Show when={props.open()}>
      <Portal>
        <div
          class="fixed inset-0 z-50 flex items-center justify-center"
          onClick={(e) => { if (e.target === e.currentTarget) handleClose(); }}
        >
          <div class="absolute inset-0 bg-black/60 backdrop-blur-sm" />
          <div class="relative z-10 w-full max-w-md rounded-xl border theme-border shadow-2xl shadow-black/60 theme-surface overflow-hidden">
            <div class="flex items-center justify-between border-b theme-border px-4 py-3">
              <span class="text-[13px] font-bold theme-text">Add MCP Server</span>
              <button
                onClick={handleClose}
                class="theme-muted hover:text-primary transition-colors"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </div>
            <div class="p-4 space-y-3 overflow-y-auto max-h-[70vh]">
              <FieldRow label="Name">
                <TextInput value={name()} onInput={setName} placeholder="e.g. chrome-devtools" monospace />
              </FieldRow>
              <FieldRow label="Transport">
                <InlineSelect
                  value={transport()}
                  options={[
                    { value: "stdio", label: "stdio — subprocess" },
                    { value: "http", label: "http — HTTP streamable" },
                    { value: "sse", label: "sse — Server-Sent Events" },
                  ]}
                  onChange={(v) => setTransport(v as "stdio" | "http" | "sse")}
                />
              </FieldRow>
              <Show when={transport() === "stdio"}>
                <FieldRow label="Command">
                  <TextInput value={command()} onInput={setCommand} placeholder="npx" monospace />
                </FieldRow>
                <FieldRow label="Args">
                  <TextInput value={args()} onInput={setArgs} placeholder="-y @scope/pkg@latest" monospace />
                </FieldRow>
                <Show when={argsInvalid()}>
                  <p class="ml-24 text-[10px] text-rose-400 font-mono">Invalid args: check quotes/escaping.</p>
                </Show>
                <FieldRow label="Env">
                  <TextInput value={envText()} onInput={setEnvText} placeholder="KEY=value (one per line)" multiline rows={2} monospace />
                </FieldRow>
              </Show>
              <Show when={transport() !== "stdio"}>
                <FieldRow label="URL">
                  <TextInput value={url()} onInput={setUrl} placeholder="https://mcp.example.com" monospace />
                </FieldRow>
                <FieldRow label="Headers">
                  <TextInput value={headersText()} onInput={setHeadersText} placeholder="Authorization=Bearer xxx (one per line)" multiline rows={2} monospace />
                </FieldRow>
              </Show>
              <Show when={props.defaultRoleName}>
                <label class="flex items-center gap-2 cursor-pointer select-none pt-1">
                  <input
                    type="checkbox"
                    checked={enableForRole()}
                    onChange={(e) => setEnableForRole(e.currentTarget.checked)}
                    class="accent-indigo-400 h-3.5 w-3.5"
                  />
                  <span class="text-[11px] theme-muted">Enable for <span class="font-mono text-indigo-300">{props.defaultRoleName}</span></span>
                </label>
              </Show>
            </div>
            <div class="flex items-center justify-between border-t theme-border px-4 py-3">
              <Show when={error()}>
                <span class="text-[10px] text-rose-400 flex-1 mr-3">{error()}</span>
              </Show>
              <div class="flex gap-2 ml-auto">
                <button
                  onClick={handleClose}
                  class={`rounded-md border theme-border px-3 py-1.5 text-[11px] theme-muted hover:text-primary transition-colors ${INTERACTIVE_MOTION}`}
                >
                  Cancel
                </button>
                <button
                  onClick={() => void handleAdd()}
                  disabled={saving() || !server() || argsInvalid()}
                  class={`rounded-md border border-indigo-500/40 bg-indigo-500/10 px-3 py-1.5 text-[11px] font-semibold text-indigo-300 hover:bg-indigo-500/20 disabled:opacity-40 disabled:cursor-not-allowed transition-colors ${INTERACTIVE_MOTION}`}
                >
                  {saving() ? "Adding…" : "Add Server"}
                </button>
              </div>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}
