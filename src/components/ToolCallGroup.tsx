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
    const styles = getComputedStyle(containerEl);
    const terminalBg = styles.getPropertyValue("--ui-terminal-bg").trim() || "#0b0b0d";
    const terminalText = styles.getPropertyValue("--ui-terminal-text").trim() || "#d4d4d8";
    term = new Terminal({
      disableStdin: true,
      scrollback: 1000,
      rows: 12,
      convertEol: true,
      theme: {
        background: terminalBg,
        foreground: terminalText,
        cursor: terminalText,
      },
      fontSize: 11,
      fontFamily: "ui-monospace, 'Cascadia Code', 'Source Code Pro', Menlo, monospace",
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

type ToolCallItemProps = {
  tc: AppToolCall;
  terminals?: Record<string, TerminalEntry>;
  inlinePermission?: AppPermission | null;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
  onRejectHunk?: (rejectPrompt: string) => void;
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
      class="tool-call-item group/tc"
      classList={{ "needs-approval": showPermission() }}
    >
      <summary class="tool-call-item-summary select-none">
        <span class={`ui-tool-status-dot ${tcStatusDot(tc().status)}`} />
        <span class="theme-text font-mono tracking-tight font-medium transition-colors truncate">{tc().title || tc().toolCallId}</span>
        <Show when={terminalEntry()}>
          <Badge tone="info">term</Badge>
        </Show>
        <Show when={showPermission()}>
          <Badge tone="warning">approval</Badge>
        </Show>
        <Badge class="ml-auto">{tc().kind}</Badge>
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
      <Show when={tc().contentJson}>
        <pre class="whitespace-pre-wrap break-words border-t theme-border px-3 py-2.5 text-[11px] font-mono theme-muted bg-[var(--ui-panel-2)]">{tc().contentJson}</pre>
      </Show>
      <Show when={tc().locations && tc().locations!.length > 0}>
        <div class="border-t theme-border px-3 py-2 text-[10.5px] theme-muted bg-[var(--ui-panel-2)]">
          <div class="mb-0.5 uppercase tracking-wider text-[9px] theme-muted">Files</div>
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
      <Show when={tc().rawInputJson}>
        <pre class="whitespace-pre-wrap break-words border-t theme-border px-3 py-2.5 text-[10.5px] font-mono theme-muted bg-[var(--ui-panel-2)]">{tc().rawInputJson}</pre>
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
            <div class="border-t theme-border bg-[var(--ui-panel-2)]">
              <Show when={diffData()} fallback={
                <pre class="whitespace-pre-wrap break-words px-3 py-2.5 text-[10.5px] font-mono theme-muted">{raw()}</pre>
              }>
                {(diffs) => (
                  <div class="p-2.5">
                    <DiffViewer
                      diffs={diffs()}
                      onRejectHunk={(filePath: string, hunk: DiffHunk) => {
                        props.onRejectHunk?.(hunkToRejectPrompt(filePath, hunk));
                      }}
                    />
                  </div>
                )}
              </Show>
            </div>
          );
        }}
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

  return (
    <div class="tool-call-card" classList={{ "needs-approval": hasPendingPermission() }}>
      <button
        class="tool-call-summary select-none"
        onClick={() => setExpanded(v => !v)}
      >
        <span class={`ui-tool-status-dot ${tcStatusDot(lastTool()?.status ?? "pending")}`} />
        <span class="theme-text font-mono tracking-tight font-medium">{lastTool()?.title || "tool calls"}</span>
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
          <Badge class="ml-1">{count()} calls</Badge>
          <svg class={`w-3 h-3 theme-muted transition-transform duration-150 ${expanded() ? "rotate-180" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
        </span>
      </button>
      <Show when={expanded()}>
        <div class="tool-call-list space-y-1">
          <For each={props.tools}>{(tc, i) => (
            <ToolCallItem
              tc={tc}
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
