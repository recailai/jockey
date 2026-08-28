import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { assistantApi, runtimeProfileApi } from "../../lib/tauriApi";
import type { AssistantRuntime } from "../types";
import { RUNTIME_COLOR } from "../types";

function runtimeIcon(key: string): string {
  if (key.includes("claude")) return "◆";
  if (key.includes("gemini") || key.includes("antigravity") || key === "agy") return "◈";
  if (key.includes("codex")) return "◉";
  if (key.includes("pi")) return "◇";
  return "○";
}

function runtimeStatusColor(available: boolean) {
  return available ? "management-badge-success" : "management-badge-muted";
}

export function ExternalAgentsTab() {
  const [runtimes, setRuntimes] = createSignal<AssistantRuntime[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const [customLabel, setCustomLabel] = createSignal("");
  const [customCommand, setCustomCommand] = createSignal("");
  const [customArgs, setCustomArgs] = createSignal("");
  const [savingCustom, setSavingCustom] = createSignal(false);
  const nativeRuntimes = createMemo(() => runtimes().filter((rt) => rt.family === "native"));
  const acpRuntimes = createMemo(() => runtimes().filter((rt) => rt.family === "acp"));

  const capabilityLabels = (rt: AssistantRuntime) => [
    ["streaming", "stream"],
    ["sessionResume", "resume"],
    ["modelCatalog", "models"],
    ["mcpServers", "MCP"],
    ["permissionRequests", "permissions"],
    ["rewind", "rewind"],
  ].filter(([key]) => rt.capabilities[key]);

  const refresh = async () => {
    setLoading(true);
    setError("");
    try {
      const detected = await assistantApi.detect();
      setRuntimes(detected);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const addCustomProfile = async () => {
    const label = customLabel().trim();
    const command = customCommand().trim();
    if (!label || !command || savingCustom()) return;
    setSavingCustom(true);
    setError("");
    try {
      await runtimeProfileApi.upsert({
        label,
        command,
        args: customArgs().trim() ? customArgs().trim().split(/\s+/) : [],
      });
      setCustomLabel("");
      setCustomCommand("");
      setCustomArgs("");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingCustom(false);
    }
  };

  onMount(() => { void refresh(); });

  return (
    <div class="flex flex-col h-full overflow-auto p-5 space-y-5">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-[13px] font-bold theme-text">External Agents</h2>
          <p class="text-[11px] theme-muted mt-0.5">Detected agent runtimes available on this machine</p>
        </div>
        <button
          onClick={() => void refresh()}
          disabled={loading()}
          class="flex items-center gap-1.5 rounded-md border theme-border px-2.5 py-1.5 text-[10px] theme-muted hover:text-primary transition-colors disabled:opacity-40"
        >
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" classList={{ "animate-spin": loading() }}><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>
          Refresh
        </button>
      </div>

      <Show when={error()}>
        <div class="management-error-box">{error()}</div>
      </Show>

      <Show when={loading() && runtimes().length === 0}>
        <div class="text-[11px] theme-muted">Detecting runtimes…</div>
      </Show>

      <For each={[{ label: "Native CLI", rows: nativeRuntimes() }, { label: "ACP Agents", rows: acpRuntimes() }]}>
        {(group) => <Show when={group.rows.length > 0}>
          <div class="space-y-2">
            <div class="font-mono text-[9px] uppercase tracking-[0.18em] theme-muted">{group.label}</div>
            <For each={group.rows}>{(rt) => (
          <div class="flex items-center gap-3 rounded-xl border theme-border theme-surface px-4 py-3">
            <span class={`text-[18px] ${RUNTIME_COLOR[rt.key] ?? "runtime-color-muted"}`}>{runtimeIcon(rt.key)}</span>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <span class="text-[12px] font-semibold theme-text font-mono">{rt.label}</span>
                <span class="text-[9px] theme-muted font-mono">{rt.binary}</span>
                <span class="text-[9px] theme-muted font-mono">{rt.transport}</span>
                <Show when={rt.launchMethod}>
                  <span class="text-[9px] theme-muted font-mono">{rt.launchMethod}</span>
                </Show>
                <span class={`rounded-md border px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-widest ${runtimeStatusColor(rt.available)}`}>
                  {rt.available ? "detected" : "not found"}
                </span>
              </div>
              <Show when={rt.version}>
                <div class="text-[10px] theme-muted font-mono mt-0.5">{rt.version}</div>
              </Show>
              <Show when={!rt.available && (rt.unavailableReason || rt.installHint)}>
                <div class="text-[9.5px] theme-muted mt-0.5">{rt.unavailableReason ?? rt.installHint}</div>
              </Show>
              <div class="flex flex-wrap gap-1 mt-1">
                <span class="rounded border theme-border px-1 py-0.5 text-[8px] font-mono theme-muted">{rt.profileId}</span>
                <For each={capabilityLabels(rt)}>{([, label]) => (
                  <span class="rounded border border-emerald-500/30 px-1 py-0.5 text-[8px] font-mono text-emerald-400">{label}</span>
                )}</For>
              </div>
            </div>
          </div>
            )}</For>
          </div>
        </Show>}
      </For>

      <div class="rounded-lg border theme-border bg-[var(--ui-panel-2)] px-4 py-3 text-[10.5px] theme-muted leading-relaxed">
        Runtimes are tied to installed binaries. To add a new runtime, install the corresponding CLI tool
        (e.g. <span class="font-mono runtime-color-claude">claude</span>, <span class="font-mono runtime-color-antigravity">agy</span>,{" "}
        <span class="font-mono runtime-color-codex">codex</span>) and click Refresh.
      </div>

      <div class="rounded-xl border theme-border theme-surface px-4 py-3 space-y-2">
        <div class="font-mono text-[9px] uppercase tracking-[0.18em] theme-muted">Add custom ACP agent</div>
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
          <input value={customLabel()} onInput={(e) => setCustomLabel(e.currentTarget.value)} placeholder="Label" class="rounded-md border theme-border bg-transparent px-2 py-1.5 text-[10px] theme-text outline-none" />
          <input value={customCommand()} onInput={(e) => setCustomCommand(e.currentTarget.value)} placeholder="Command (e.g. acpx)" class="rounded-md border theme-border bg-transparent px-2 py-1.5 text-[10px] theme-text font-mono outline-none" />
          <input value={customArgs()} onInput={(e) => setCustomArgs(e.currentTarget.value)} placeholder="Args (optional)" class="rounded-md border theme-border bg-transparent px-2 py-1.5 text-[10px] theme-text font-mono outline-none" />
        </div>
        <button
          onClick={() => void addCustomProfile()}
          disabled={savingCustom() || !customLabel().trim() || !customCommand().trim()}
          class="rounded-md border theme-border px-2.5 py-1.5 text-[10px] theme-muted hover:text-primary transition-colors disabled:opacity-40"
        >
          {savingCustom() ? "Saving…" : "Register ACP agent"}
        </button>
      </div>
    </div>
  );
}
