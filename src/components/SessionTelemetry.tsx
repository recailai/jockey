import { For, Show, createMemo } from "solid-js";
import { AlertTriangle, Info } from "lucide-solid";
import type { Accessor } from "solid-js";
import type { AppSession, RuntimeCapabilities } from "./types";

/**
 * Token accounting and provider advisories. Every supported CLI reports usage in some
 * form; before this it was parsed and thrown away, so context pressure and spend were
 * invisible. Fields the provider does not report stay hidden rather than showing zero.
 */
export default function SessionTelemetry(props: { activeSession: Accessor<AppSession | null>; capabilities?: Accessor<RuntimeCapabilities | undefined> }) {
  const usage = () => props.activeSession()?.usage ?? null;
  const notices = () => props.activeSession()?.notices ?? [];

  const pressure = createMemo(() => {
    const u = usage();
    if (!u?.contextWindow || !u.totalTokens) return null;
    return Math.min(100, Math.round((u.totalTokens / u.contextWindow) * 100));
  });

  const fmtTokens = (n: number) =>
    n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1_000 ? `${(n / 1_000).toFixed(1)}k` : `${n}`;

  return (
    <>
      <Show when={notices().length > 0}>
        <div class="my-2 space-y-1">
          <For each={notices().slice(-3)}>
            {(notice) => (
              <div
                class="flex items-start gap-2 rounded-lg border px-2.5 py-1.5 text-[11px]"
                classList={{
                  "border-amber-500/25 bg-amber-500/10 text-amber-300": notice.level === "warning",
                  "theme-border theme-surface theme-muted": notice.level !== "warning",
                }}
              >
                {notice.level === "warning" ? (
                  <AlertTriangle size={12} class="mt-px shrink-0" />
                ) : (
                  <Info size={12} class="mt-px shrink-0" />
                )}
                <span class="min-w-0 flex-1">{notice.text}</span>
                <Show when={notice.code}>
                  <span class="shrink-0 font-mono text-[9px] opacity-60">{notice.code}</span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={props.capabilities?.()?.usage !== false}>
        <Show when={usage()}>
          {(u) => (
          <div class="my-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 px-1 font-mono text-[10px] theme-muted">
            <Show when={u().inputTokens !== null}>
              <span title="Input tokens">in {fmtTokens(u().inputTokens!)}</span>
            </Show>
            <Show when={u().outputTokens !== null}>
              <span title="Output tokens">out {fmtTokens(u().outputTokens!)}</span>
            </Show>
            <Show when={u().reasoningTokens !== null && u().reasoningTokens! > 0}>
              <span title="Reasoning tokens">think {fmtTokens(u().reasoningTokens!)}</span>
            </Show>
            <Show when={u().cacheReadTokens !== null && u().cacheReadTokens! > 0}>
              <span title="Tokens served from cache">cached {fmtTokens(u().cacheReadTokens!)}</span>
            </Show>
            <Show when={pressure() !== null}>
              <span
                title={`${u().totalTokens} of ${u().contextWindow} context tokens used`}
                classList={{ "text-amber-400": (pressure() ?? 0) >= 75 }}
              >
                ctx {pressure()}%
              </span>
            </Show>
            <Show when={u().costUsd !== null}>
              <span title="Cost for this session">${u().costUsd!.toFixed(4)}</span>
            </Show>
          </div>
          )}
        </Show>
      </Show>
    </>
  );
}
