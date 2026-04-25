import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { AppPermission, AppToolCall, TerminalEntry } from "./types";
import { INTERACTIVE_MOTION } from "./types";

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

type ToolCallItemProps = {
  tc: AppToolCall;
  terminals?: Record<string, TerminalEntry>;
  inlinePermission?: AppPermission | null;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
};

function ToolCallItem(props: ToolCallItemProps) {
  const tc = () => props.tc;
  const terminalEntry = () => {
    const tid = terminalIdOf(tc());
    if (!tid) return null;
    return props.terminals?.[tid] ?? null;
  };
  const showPermission = () =>
    !!props.inlinePermission && tc().status === "pending";

  const [remember, setRemember] = createSignal(false);

  const hasRememberableOptions = () =>
    !!props.inlinePermission?.options.some((o) => o.kind === "allow_always");

  return (
    <details
      class="group/tc rounded-lg border theme-border theme-surface overflow-hidden transition-all duration-200 hover:bg-[var(--ui-surface-muted)] hover:border-[var(--ui-border-strong)]"
      classList={{ "border-amber-500/50 bg-amber-500/5": showPermission() }}
    >
      <summary class="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-[11px] theme-muted select-none">
        <span class={`h-1.5 w-1.5 shrink-0 rounded-full shadow-[0_0_6px_rgba(0,0,0,0.4)] ${tcStatusDot(tc().status)}`} />
        <span class="theme-text font-mono tracking-tight font-medium group-hover/tc:text-white transition-colors truncate">{tc().title || tc().toolCallId}</span>
        <Show when={terminalEntry()}>
          <span class="shrink-0 rounded-md border border-indigo-500/30 bg-indigo-500/10 px-1.5 py-[1px] text-[9px] font-bold uppercase tracking-widest text-indigo-300">
            term
          </span>
        </Show>
        <Show when={showPermission()}>
          <span class="shrink-0 rounded-md border border-amber-500/40 bg-amber-500/15 px-1.5 py-[1px] text-[9px] font-bold uppercase tracking-widest text-amber-300">
            approval
          </span>
        </Show>
        <span class="ml-auto text-[9px] theme-muted uppercase tracking-widest font-bold bg-[var(--ui-panel-2)] px-1.5 py-0.5 rounded-md shrink-0">{tc().kind}</span>
      </summary>
      <Show when={showPermission()}>
        {(_) => {
          const perm = () => props.inlinePermission!;
          const opts = () => {
            const all = perm().options;
            if (remember()) return all.filter((o) => o.kind === "allow_always" || !o.kind || o.kind === "allow_once");
            return all.filter((o) => o.kind !== "allow_always");
          };
          return (
            <div class="border-t border-amber-500/30 bg-amber-500/8 px-3 py-2.5">
              <div class="mb-1 text-[11px] font-semibold text-amber-300">{perm().title}</div>
              <Show when={perm().description}>
                <p class="mb-2 text-[10.5px] theme-muted">{perm().description}</p>
              </Show>
              <div class="flex flex-wrap gap-2">
                <For each={opts()}>{(opt) => (
                  <button
                    class={`min-h-7 rounded border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1 text-[11px] text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
                    onClick={(e) => { e.preventDefault(); props.onApprove?.(opt.optionId); }}
                  >
                    {opt.title ?? opt.optionId}
                  </button>
                )}</For>
                <button
                  class={`min-h-7 rounded border border-rose-500/40 bg-rose-500/10 px-2.5 py-1 text-[11px] text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
                  onClick={(e) => { e.preventDefault(); props.onDeny?.(); }}
                >
                  Deny
                </button>
              </div>
              <Show when={hasRememberableOptions()}>
                <label class="mt-2 flex items-center gap-1.5 cursor-pointer select-none w-fit">
                  <input
                    type="checkbox"
                    checked={remember()}
                    onChange={(e) => setRemember(e.currentTarget.checked)}
                    class="accent-amber-400 h-3 w-3"
                  />
                  <span class="text-[10px] theme-muted">Remember my choice</span>
                </label>
              </Show>
            </div>
          );
        }}
      </Show>
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
            <button
              type="button"
              class={`block w-full text-left font-mono break-all hover:text-indigo-300 transition-colors ${props.onFileClick ? "cursor-pointer" : "cursor-default"}`}
              onClick={() => props.onFileClick?.(loc.path, tc().kind)}
            >
              {loc.path}{loc.line ? `:${loc.line}` : ""}
            </button>
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
  pendingPermission?: AppPermission | null;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
}) {
  const hasPendingPermission = () => !!props.pendingPermission;
  const [expanded, setExpanded] = createSignal(false);

  createEffect(() => {
    if (hasPendingPermission()) setExpanded(true);
  });

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

  const pendingToolCallIndex = createMemo(() => {
    if (!props.pendingPermission) return -1;
    for (let i = props.tools.length - 1; i >= 0; i--) {
      if (props.tools[i].status === "pending") return i;
    }
    return props.tools.length - 1;
  });

  return (
    <div
      class="rounded-xl border theme-border theme-panel overflow-hidden shadow-sm my-1.5"
      classList={{ "border-amber-500/40": hasPendingPermission() }}
    >
      <button
        class="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-[11.5px] theme-text select-none hover:bg-[var(--ui-accent-soft)] transition-colors"
        onClick={() => setExpanded(v => !v)}
      >
        <span class={`h-2 w-2 shrink-0 rounded-full shadow-[0_0_8px_rgba(0,0,0,0.5)] ${tcStatusDot(lastTool()?.status ?? "pending")}`} />
        <span class="theme-text font-mono tracking-tight font-medium">{lastTool()?.title || "tool calls"}</span>
        <span class="flex items-center gap-1.5 ml-auto">
          <Show when={hasPendingPermission()}>
            <span class="flex items-center gap-1 text-[9px] text-amber-300 rounded-md border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 font-bold uppercase tracking-widest">
              <span class="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse inline-block" />
              approval
            </span>
          </Show>
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
          <Show when={statusCounts().pending > 0 && !hasPendingPermission()}>
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
          <For each={props.tools}>{(tc, i) => (
            <ToolCallItem
              tc={tc}
              terminals={props.terminals}
              inlinePermission={i() === pendingToolCallIndex() ? props.pendingPermission : null}
              onApprove={props.onApprove}
              onDeny={props.onDeny}
              onFileClick={props.onFileClick}
            />
          )}</For>
        </div>
      </Show>
    </div>
  );
}
