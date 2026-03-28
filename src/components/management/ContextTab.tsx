import { invoke } from "@tauri-apps/api/core";
import { For, Show, createEffect, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession } from "../types";
import { INTERACTIVE_MOTION } from "../types";
import type { ContextEntry } from "./primitives";

export function ContextTab(props: { activeSession: Accessor<AppSession | null> }) {
  const [entries, setEntries] = createSignal<ContextEntry[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [err, setErr] = createSignal<string | null>(null);

  const load = async () => {
    const sid = props.activeSession()?.id ?? null;
    if (!sid) {
      setEntries([]);
      setErr(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    setErr(null);
    try {
      const res = await invoke<{ ok: boolean; payload: { entries?: ContextEntry[] } }>("apply_chat_command", {
        input: "/app_context list",
        appSessionId: sid,
      });
      setEntries(res.payload.entries ?? []);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  };

  createEffect(() => {
    void props.activeSession()?.id;
    void load();
  });

  return (
    <div class="flex h-full flex-col overflow-hidden">
      <div class="flex items-center justify-between border-b border-white/[0.04] px-5 py-2.5">
        <div class="flex min-w-0 items-center gap-2">
          <span class="font-mono text-[10px] font-semibold uppercase tracking-widest text-zinc-500">Session Context</span>
          <span class="truncate font-mono text-[10px] text-zinc-700">{props.activeSession()?.title ?? "no_session"}</span>
        </div>
        <button
          onClick={() => void load()}
          class={`min-h-6 rounded border border-zinc-800 px-2 py-0.5 text-[10px] text-zinc-600 hover:border-zinc-700 hover:text-zinc-300 ${INTERACTIVE_MOTION}`}
        >
          refresh
        </button>
      </div>
      <div class="flex-1 overflow-y-auto">
        <Show when={loading()}>
          <div class="px-5 py-4 font-mono text-[10px] text-zinc-600">loading…</div>
        </Show>
        <Show when={err()}>
          <div class="px-5 py-4 font-mono text-[10px] text-rose-500">{err()}</div>
        </Show>
        <Show when={!loading() && !err() && entries().length === 0}>
          <div class="px-5 py-4 font-mono text-[10px] text-zinc-600">no context entries for active session</div>
        </Show>
        <Show when={!loading() && entries().length > 0}>
          <table class="w-full border-collapse font-mono text-[10px]">
            <thead>
              <tr class="border-b border-white/[0.04] text-left text-zinc-600">
                <th class="px-5 py-2 font-semibold uppercase tracking-widest">scope</th>
                <th class="px-5 py-2 font-semibold uppercase tracking-widest">key</th>
                <th class="px-5 py-2 font-semibold uppercase tracking-widest">value</th>
              </tr>
            </thead>
            <tbody>
              <For each={entries()}>
                {(e) => (
                  <tr class="border-b border-white/[0.03] hover:bg-white/[0.02]">
                    <td class="px-5 py-2 text-zinc-500">{e.scope}</td>
                    <td class="px-5 py-2 text-amber-400/80">{e.key}</td>
                    <td class="px-5 py-2 max-w-[240px] truncate text-zinc-300" title={e.value}>{e.value}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </div>
    </div>
  );
}
