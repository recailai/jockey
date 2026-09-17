import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { AppPermission, AppToolCall, TerminalEntry } from "./types";
import { DiffViewer } from "./DiffViewer";
import { parseDiff, isDiffLike, hunkToRejectPrompt } from "../lib/diffParser";
import type { DiffHunk } from "../lib/diffParser";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { Badge, Button, Switch as UiSwitch } from "./ui";

function tcStatusDot(status: string): string {
  if (status === "success" || status === "completed") return "ui-tool-status-success";
  if (status === "failure" || status === "error") return "ui-tool-status-danger";
  return "ui-tool-status-warning animate-pulse";
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
  let containerEl: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let lastOutput = "";

  const exitLabel = () => {
    const ex = entry().exitStatus;
    if (!ex) return null;
    if (ex.signal) return `signal ${ex.signal}`;
    if (typeof ex.exitCode === "number") return `exit ${ex.exitCode}`;
    return "done";
  };
  const exitTone = (): "success" | "warning" | "danger" => {
    const ex = entry().exitStatus;
    if (!ex) return "warning";
    if (ex.signal) return "danger";
    if (ex.exitCode === 0) return "success";
    return "danger";
  };

  onMount(() => {
    if (!containerEl) return;
    const styles = getComputedStyle(document.documentElement);
    const getVar = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
    term = new Terminal({
      disableStdin: true,
      scrollback: 1000,
      rows: 12,
      convertEol: true,
      drawBoldTextInBrightColors: true,
      cursorStyle: "bar",
      cursorWidth: 2,
      theme: {
        background: getVar("--ui-terminal-bg", "#0c0c0e"),
        foreground: getVar("--ui-terminal-text", "#f4f4f5"),
        cursor: getVar("--ui-terminal-text", "#f4f4f5"),
        selectionBackground: getVar("--ui-terminal-selection", "#3f3f46"),
        black: getVar("--ui-terminal-black", "#18181b"),
        red: getVar("--ui-terminal-red", "#ef596f"),
        green: getVar("--ui-terminal-green", "#89ca78"),
        yellow: getVar("--ui-terminal-yellow", "#e5c07b"),
        blue: getVar("--ui-terminal-blue", "#61afef"),
        magenta: getVar("--ui-terminal-magenta", "#c678dd"),
        cyan: getVar("--ui-terminal-cyan", "#56b6c2"),
        white: getVar("--ui-terminal-white", "#abb2bf"),
        brightBlack: getVar("--ui-terminal-brightBlack", "#5c6370"),
        brightRed: getVar("--ui-terminal-brightRed", "#ef596f"),
        brightGreen: getVar("--ui-terminal-brightGreen", "#89ca78"),
        brightYellow: getVar("--ui-terminal-brightYellow", "#e5c07b"),
        brightBlue: getVar("--ui-terminal-brightBlue", "#61afef"),
        brightMagenta: getVar("--ui-terminal-brightMagenta", "#c678dd"),
        brightCyan: getVar("--ui-terminal-brightCyan", "#56b6c2"),
        brightWhite: getVar("--ui-terminal-brightWhite", "#ffffff"),
      },
      fontSize: 11,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerEl);
    fitAddon.fit();

    const output = entry().output;
    if (output) {
      term.write(output);
      lastOutput = output;
    }
  });

  createEffect(() => {
    const output = entry().output;
    if (!term || output === lastOutput) return;
    if (output.startsWith(lastOutput)) {
      term.write(output.slice(lastOutput.length));
    } else {
      term.reset();
      term.write(output);
    }
    lastOutput = output;
    fitAddon?.fit();
  });

  onCleanup(() => { term?.dispose(); });

  return (
    <div class="inline-terminal">
      <div class="flex items-center justify-between px-3 py-1.5 text-[9.5px] theme-muted uppercase tracking-widest border-b theme-border">
        <span class="font-mono truncate">
          {entry().label ?? "terminal"}
          <Show when={entry().cwd}>
            <span class="opacity-60"> · {entry().cwd}</span>
          </Show>
        </span>
        <Show when={exitLabel()}>
          <Badge tone={exitTone()}>
            {exitLabel()}
          </Badge>
        </Show>
      </div>
      <div
        ref={containerEl}
        class="px-1 py-1 overflow-hidden"
        style={{ "max-height": "280px" }}
      />
      <Show when={!entry().output}>
        <div class="px-3 py-2 text-[11px] font-mono theme-muted italic">(waiting for output...)</div>
      </Show>
    </div>
  );
}

import { formatToolDisplay } from "../lib/toolDisplay";

type ToolCallItemProps = {
  tc: AppToolCall;
  cwd?: string | null;
  terminals?: Record<string, TerminalEntry>;
  inlinePermission?: AppPermission | null;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
  onRejectHunk?: (rejectPrompt: string) => void;
};

function categoryBadgeClass(category: string): string {
  switch (category) {
    case "shell":
      return "bg-amber-500/15 text-amber-400 border-amber-500/30";
    case "read":
      return "bg-blue-500/15 text-blue-400 border-blue-500/30";
    case "edit":
    case "write":
      return "bg-emerald-500/15 text-emerald-400 border-emerald-500/30";
    case "search":
      return "bg-purple-500/15 text-purple-400 border-purple-500/30";
    case "fetch":
      return "bg-cyan-500/15 text-cyan-400 border-cyan-500/30";
    case "task":
      return "bg-rose-500/15 text-rose-400 border-rose-500/30";
    default:
      return "bg-[var(--ui-surface-muted)] text-[var(--ui-muted)] border-[var(--ui-border)]";
  }
}

function ToolCallItem(props: ToolCallItemProps) {
  const tc = () => props.tc;
  const rawOutputText = () => tc().rawOutputJson;
  const terminalEntry = () => {
    const tid = terminalIdOf(tc());
    if (!tid) return null;
    return props.terminals?.[tid] ?? null;
  };
  const showPermission = () =>
    !!props.inlinePermission && tc().status === "pending";

  const [remember, setRemember] = createSignal(false);
  const displayInfo = createMemo(() => formatToolDisplay(tc(), props.cwd));

  const hasRememberableOptions = () =>
    !!props.inlinePermission?.options.some((o) => o.kind === "allow_always");

  return (
    <details
      class="tool-call-item group/tc"
      classList={{ "needs-approval": showPermission() }}
    >
      <summary class="tool-call-item-summary select-none flex items-center gap-2 py-1.5 px-3">
        <span class={`ui-tool-status-dot shrink-0 ${tcStatusDot(tc().status)}`} />
        
        {/* Canonical action badge */}
        <span class={`font-mono text-[9px] font-semibold uppercase px-1.5 py-0.5 rounded border shrink-0 ${categoryBadgeClass(displayInfo().category)}`}>
          {displayInfo().categoryBadge}
        </span>

        {/* Concise target summary (file path / command / query) */}
        <span
          class="theme-text font-mono text-[11px] tracking-tight truncate flex-1 min-w-0"
          title={displayInfo().targetSummary || tc().title || tc().toolCallId}
        >
          {displayInfo().targetSummary || tc().title || tc().toolCallId}
        </span>

        <Show when={tc().parentId}>
          <Badge tone="info">subagent</Badge>
        </Show>
        <Show when={terminalEntry()}>
          <Badge tone="info">term</Badge>
        </Show>
        <Show when={showPermission()}>
          <Badge tone="warning">approval</Badge>
        </Show>
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
            <div class="tool-permission-block">
              <div class="mb-1 text-[11px] font-semibold text-[var(--ui-state-warning-text)]">{perm().title}</div>
              <Show when={perm().description}>
                <p class="mb-2 text-[10.5px] theme-muted">{perm().description}</p>
              </Show>
              <div class="flex flex-wrap gap-2">
                <For each={opts()}>{(opt) => (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={(e) => { e.preventDefault(); props.onApprove?.(opt.optionId); }}
                  >
                    {opt.title ?? opt.optionId}
                  </Button>
                )}</For>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={(e) => { e.preventDefault(); props.onDeny?.(); }}
                >
                  Deny
                </Button>
              </div>
              <Show when={hasRememberableOptions()}>
                <label class="mt-2 flex items-center gap-1.5 cursor-pointer select-none w-fit">
                  <UiSwitch
                    checked={remember()}
                    onChange={setRemember}
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
      <Show when={tc().locations && tc().locations!.length > 0}>
        <div class="border-t theme-border px-3 py-2 text-[10.5px] theme-muted bg-[var(--ui-panel-2)]">
          <div class="mb-0.5 uppercase tracking-wider text-[9px] theme-muted font-semibold">Referenced Files</div>
          <For each={tc().locations}>{(loc) => (
            <button
              type="button"
              class={`block w-full text-left font-mono break-all hover:text-[var(--ui-accent)] transition-colors ${props.onFileClick ? "cursor-pointer" : "cursor-default"}`}
              onClick={() => props.onFileClick?.(loc.path, tc().kind)}
            >
              {loc.path}{loc.line ? `:${loc.line}` : ""}
            </button>
          )}</For>
        </div>
      </Show>
      <Show when={tc().outputLog}>
        <div class="px-2 pb-1.5">
          <div class="text-[9px] uppercase tracking-wide theme-muted">output</div>
          <pre class="mt-0.5 max-h-48 overflow-y-auto whitespace-pre-wrap break-words rounded bg-[var(--ui-surface-muted)] p-2 font-mono text-[10px] theme-muted">{tc().outputLog}</pre>
        </div>
      </Show>
      <Show when={tc().rawOutputJson}>
        {(_) => {
          const raw = () => tc().rawOutputJson!;
          const diffData = () => {
            try {
              const parsed = JSON.parse(raw());
              const text = typeof parsed === "string" ? parsed
                : typeof parsed?.patch === "string" ? parsed.patch
                : typeof parsed?.diff === "string" ? parsed.diff
                : null;
              if (text && isDiffLike(text)) return parseDiff(text);
            } catch {
              if (isDiffLike(raw())) return parseDiff(raw());
            }
            return null;
          };
          return (
            <Show when={diffData()}>
              {(diffs) => (
                <div class="border-t theme-border bg-[var(--ui-panel-2)] p-2.5">
                  <DiffViewer
                    diffs={diffs()}
                    onRejectHunk={(filePath: string, hunk: DiffHunk) => {
                      props.onRejectHunk?.(hunkToRejectPrompt(filePath, hunk));
                    }}
                  />
                </div>
              )}
            </Show>
          );
        }}
      </Show>
      {/* Discreet Debug Payload for raw JSON */}
      <Show when={tc().rawInputJson || (rawOutputText() && !isDiffLike(rawOutputText()!)) || tc().contentJson}>
        <details class="border-t theme-border bg-[var(--ui-panel-2)]">
          <summary class="px-3 py-1.5 text-[9px] font-mono theme-muted cursor-pointer hover:text-[var(--ui-text)] transition-colors select-none">
            Debug Payload ({tc().toolCallId})
          </summary>
          <div class="p-2 space-y-2 border-t theme-border">
            <Show when={tc().rawInputJson}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Input:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().rawInputJson}</pre>
              </div>
            </Show>
            <Show when={rawOutputText() && !isDiffLike(rawOutputText()!)}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Output:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().rawOutputJson}</pre>
              </div>
            </Show>
            <Show when={tc().contentJson}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Content:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().contentJson}</pre>
              </div>
            </Show>
          </div>
        </details>
      </Show>
    </details>
  );
}

export function ToolCallGroup(props: {
  tools: AppToolCall[];
  streaming: boolean;
  cwd?: string | null;
  terminals?: Record<string, TerminalEntry>;
  pendingPermission?: AppPermission | null;
  pendingCount?: number;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
  onRejectHunk?: (rejectPrompt: string) => void;
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

  const groupSummaryLabel = createMemo(() => {
    if (props.tools.length === 0) return "tool calls";
    if (props.tools.length === 1) {
      const display = formatToolDisplay(props.tools[0], props.cwd);
      return `${display.actionLabel}: ${display.targetSummary}`;
    }
    const categories = Array.from(
      new Set(props.tools.map((t) => formatToolDisplay(t, props.cwd).categoryBadge)),
    );
    return `Ran ${props.tools.length} tools (${categories.join(", ")})`;
  });

  return (
    <div class="tool-call-card" classList={{ "needs-approval": hasPendingPermission() }}>
      <button
        class="tool-call-summary select-none"
        onClick={() => setExpanded(v => !v)}
      >
        <span class={`ui-tool-status-dot ${tcStatusDot(lastTool()?.status ?? "pending")}`} />
        <span class="theme-text font-mono tracking-tight font-medium truncate max-w-[340px]">
          {groupSummaryLabel()}
        </span>
        <span class="flex items-center gap-1.5 ml-auto">
          <Show when={hasPendingPermission()}>
            <span class="tool-summary-badge is-warning">
              <span class="ui-tool-status-dot ui-tool-status-warning animate-pulse" />
              approval
            </span>
          </Show>
          <Show when={statusCounts().success > 0}>
            <span class="tool-summary-badge is-success">
              <span class="ui-tool-status-dot ui-tool-status-success" />
              {statusCounts().success}
            </span>
          </Show>
          <Show when={statusCounts().error > 0}>
            <span class="tool-summary-badge is-danger">
              <span class="ui-tool-status-dot ui-tool-status-danger" />
              {statusCounts().error}
            </span>
          </Show>
          <Show when={statusCounts().pending > 0 && !hasPendingPermission()}>
            <span class="tool-summary-badge is-warning">
              <span class="ui-tool-status-dot ui-tool-status-warning animate-pulse" />
              {statusCounts().pending}
            </span>
          </Show>
          <Badge class="ml-1">{count()} {count() === 1 ? "call" : "calls"}</Badge>
          <svg class={`w-3 h-3 theme-muted transition-transform duration-150 ${expanded() ? "rotate-180" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
        </span>
      </button>
      <Show when={expanded()}>
        <div class="tool-call-list space-y-1">
          <For each={props.tools}>{(tc, i) => (
            <ToolCallItem
              tc={tc}
              cwd={props.cwd}
              terminals={props.terminals}
              inlinePermission={i() === pendingToolCallIndex() ? props.pendingPermission : null}
              onApprove={props.onApprove}
              onDeny={props.onDeny}
              onFileClick={props.onFileClick}
              onRejectHunk={props.onRejectHunk}
            />
          )}</For>
        </div>
      </Show>
    </div>
  );
}
