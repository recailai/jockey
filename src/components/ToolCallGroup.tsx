import { For, Show, createMemo, createSignal } from "solid-js";
import type { AppToolCall, TerminalEntry } from "./types";

function tcStatusDot(status: string): string {
  if (status === "success" || status === "completed") return "bg-emerald-400 shadow-emerald-400/40";
  if (status === "failure" || status === "error") return "bg-rose-400 shadow-rose-400/40";
  return "bg-amber-400 animate-pulse shadow-amber-400/40";
}

/** Extract `{ terminalId }` from a tool call's terminalMeta.terminalInfo, if any. */
function terminalIdOf(tc: AppToolCall): string | null {
  const meta = tc.terminalMeta;
  if (!meta || typeof meta !== "object") return null;
  const info = (meta as Record<string, unknown>).terminalInfo as
    | { terminalId?: string }
    | undefined;
  return info?.terminalId ?? null;
}

function TerminalView(props: { entry: TerminalEntry }) {
  const entry = () => props.entry;
  const exitLabel = () => {
    const ex = entry().exitStatus;
    if (!ex) return null;
    if (ex.signal) return `signal ${ex.signal}`;
    if (typeof ex.exitCode === "number") return `exit ${ex.exitCode}`;
    return "done";
  };
  const exitTone = () => {
    const ex = entry().exitStatus;
    if (!ex) return "bg-amber-400/15 text-amber-300 border-amber-500/30";
    if (ex.signal) return "bg-rose-500/15 text-rose-300 border-rose-500/30";
    if (ex.exitCode === 0) return "bg-emerald-500/15 text-emerald-300 border-emerald-500/30";
    return "bg-rose-500/15 text-rose-300 border-rose-500/30";
  };
  return (
    <div class="border-t theme-border bg-[#0b0b0d]">
      <div class="flex items-center justify-between px-3 py-1.5 text-[9.5px] theme-muted uppercase tracking-widest border-b theme-border">
        <span class="font-mono truncate">
          {entry().label ?? "terminal"}
          <Show when={entry().cwd}>
            <span class="theme-muted/70"> · {entry().cwd}</span>
          </Show>
        </span>
        <Show when={exitLabel()}>
          <span class={`ml-2 shrink-0 rounded-md border px-1.5 py-[1px] text-[9px] font-bold ${exitTone()}`}>
            {exitLabel()}
          </span>
        </Show>
      </div>
      <pre class="whitespace-pre-wrap break-words px-3 py-2 text-[11px] font-mono leading-[1.35] text-zinc-200 max-h-[360px] overflow-auto">
        {entry().output || <span class="theme-muted italic">(waiting for output...)</span>}
      </pre>
    </div>
  );
}

function ToolCallItem(props: { tc: AppToolCall; terminals?: Record<string, TerminalEntry> }) {
  const tc = () => props.tc;
  const terminalEntry = () => {
    const tid = terminalIdOf(tc());
    if (!tid) return null;
    return props.terminals?.[tid] ?? null;
  };
  return (
    <details class="group/tc rounded-lg border theme-border theme-surface overflow-hidden transition-all duration-200 hover:bg-[var(--ui-surface-muted)] hover:border-[var(--ui-border-strong)]">
      <summary class="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-[11px] theme-muted select-none">
        <span class={`h-1.5 w-1.5 shrink-0 rounded-full shadow-[0_0_6px_rgba(0,0,0,0.4)] ${tcStatusDot(tc().status)}`} />
        <span class="theme-text font-mono tracking-tight font-medium group-hover/tc:text-white transition-colors truncate">{tc().title || tc().toolCallId}</span>
        <Show when={terminalEntry()}>
          <span class="shrink-0 rounded-md border border-indigo-500/30 bg-indigo-500/10 px-1.5 py-[1px] text-[9px] font-bold uppercase tracking-widest text-indigo-300">
            term
          </span>
        </Show>
        <span class="ml-auto text-[9px] theme-muted uppercase tracking-widest font-bold bg-[var(--ui-panel-2)] px-1.5 py-0.5 rounded-md shrink-0">{tc().kind}</span>
      </summary>
      <Show when={terminalEntry()}>
        <TerminalView entry={terminalEntry()!} />
      </Show>
      <Show when={tc().contentJson}>
        <pre class="whitespace-pre-wrap break-words border-t theme-border px-3 py-2.5 text-[11px] font-mono theme-muted bg-[var(--ui-panel-2)]">{tc().contentJson}</pre>
      </Show>
      <Show when={tc().locations && tc().locations!.length > 0}>
        <div class="border-t theme-border px-3 py-2 text-[10.5px] theme-muted bg-[var(--ui-panel-2)]">
          <div class="mb-0.5 uppercase tracking-wider text-[9px] theme-muted">Files</div>
          <For each={tc().locations}>{(loc) => (
            <div class="font-mono break-all">{loc.path}{loc.line ? `:${loc.line}` : ""}</div>
          )}</For>
        </div>
      </Show>
      <Show when={tc().rawInputJson}>
        <pre class="whitespace-pre-wrap break-words border-t theme-border px-3 py-2.5 text-[10.5px] font-mono theme-muted bg-[var(--ui-panel-2)]">{tc().rawInputJson}</pre>
      </Show>
      <Show when={tc().rawOutputJson}>
        <pre class="whitespace-pre-wrap break-words border-t theme-border px-3 py-2.5 text-[10.5px] font-mono theme-muted bg-[var(--ui-panel-2)]">{tc().rawOutputJson}</pre>
      </Show>
    </details>
  );
}

export function ToolCallGroup(props: {
  tools: AppToolCall[];
  streaming: boolean;
  terminals?: Record<string, TerminalEntry>;
}) {
  const [expanded, setExpanded] = createSignal(false);

  const count = () => props.tools.length;
  const lastTool = () => props.tools[props.tools.length - 1];
  const statusCounts = createMemo(() => {
    let success = 0, error = 0;
    for (const t of props.tools) {
      if (t.status === "success" || t.status === "completed") success++;
      else if (t.status === "failure" || t.status === "error") error++;
    }
    return { success, error, pending: props.tools.length - success - error };
  });

  return (
    <div class="rounded-xl border theme-border theme-panel overflow-hidden shadow-sm my-1.5">
      <button
        class="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-[11.5px] theme-text select-none hover:bg-[var(--ui-accent-soft)] transition-colors"
        onClick={() => setExpanded(v => !v)}
      >
        <span class={`h-2 w-2 shrink-0 rounded-full shadow-[0_0_8px_rgba(0,0,0,0.5)] ${tcStatusDot(lastTool()?.status ?? "pending")}`} />
        <span class="theme-text font-mono tracking-tight font-medium">{lastTool()?.title || "tool calls"}</span>
        <span class="flex items-center gap-1.5 ml-auto">
          <Show when={statusCounts().success > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-emerald-400">
              <span class="h-1.5 w-1.5 rounded-full bg-emerald-400 inline-block" />
              {statusCounts().success}
            </span>
          </Show>
          <Show when={statusCounts().error > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-rose-400">
              <span class="h-1.5 w-1.5 rounded-full bg-rose-400 inline-block" />
              {statusCounts().error}
            </span>
          </Show>
          <Show when={statusCounts().pending > 0}>
            <span class="flex items-center gap-0.5 text-[9px] text-amber-400">
              <span class="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse inline-block" />
              {statusCounts().pending}
            </span>
          </Show>
          <span class="text-[9px] theme-muted font-bold tracking-widest uppercase bg-[var(--ui-panel-2)] px-1.5 py-0.5 rounded-md ml-1">{count()} calls</span>
          <svg class={`w-3 h-3 text-zinc-500 transition-transform duration-150 ${expanded() ? "rotate-180" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
        </span>
      </button>
      <Show when={expanded()}>
        <div class="border-t border-white/[0.05] px-2 py-1.5 space-y-1">
          <For each={props.tools}>{(tc) => (
            <ToolCallItem tc={tc} terminals={props.terminals} />
          )}</For>
        </div>
      </Show>
    </div>
  );
}
