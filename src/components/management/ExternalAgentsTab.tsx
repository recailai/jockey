import { For, Show, createSignal, onMount } from "solid-js";
import { assistantApi } from "../../lib/tauriApi";
import type { AssistantRuntime } from "../types";
import { RUNTIME_COLOR } from "../types";

function runtimeIcon(key: string): string {
  if (key.includes("claude")) return "◆";
  if (key.includes("gemini")) return "◈";
  if (key.includes("codex")) return "◉";
  return "○";
}

function runtimeStatusColor(available: boolean) {
  return available ? "management-badge-success" : "management-badge-muted";
}

export function ExternalAgentsTab() {
  const [runtimes, setRuntimes] = createSignal<AssistantRuntime[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");

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

      <div class="space-y-2">
        <For each={runtimes()}>{(rt) => (
          <div class="flex items-center gap-3 rounded-xl border theme-border theme-surface px-4 py-3">
            <span class={`text-[18px] ${RUNTIME_COLOR[rt.key] ?? "runtime-color-muted"}`}>{runtimeIcon(rt.key)}</span>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <span class="text-[12px] font-semibold theme-text font-mono">{rt.label}</span>
                <span class="text-[9px] theme-muted font-mono">{rt.binary}</span>
                <span class={`rounded-md border px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-widest ${runtimeStatusColor(rt.available)}`}>
                  {rt.available ? "detected" : "not found"}
                </span>
              </div>
              <Show when={rt.version}>
                <div class="text-[10px] theme-muted font-mono mt-0.5">{rt.version}</div>
              </Show>
              <Show when={!rt.available && rt.installHint}>
                <div class="text-[9.5px] theme-muted mt-0.5">{rt.installHint}</div>
              </Show>
            </div>
          </div>
        )}</For>
      </div>

      <div class="rounded-lg border theme-border bg-[var(--ui-panel-2)] px-4 py-3 text-[10.5px] theme-muted leading-relaxed">
        Runtimes are tied to installed binaries. To add a new runtime, install the corresponding CLI tool
        (e.g. <span class="font-mono runtime-color-claude">claude</span>, <span class="font-mono runtime-color-gemini">gemini</span>,{" "}
        <span class="font-mono runtime-color-codex">codex</span>) and click Refresh.
      </div>
    </div>
  );
}
