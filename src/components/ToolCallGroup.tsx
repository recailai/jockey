import { For, Match, Show, Switch, createContext, createEffect, createMemo, createSignal, onCleanup, onMount, useContext } from "solid-js";
import { Check, ChevronDown, Square } from "lucide-solid";
import { PermissionBody } from "./PermissionModal";
import type { AppPermission, AppToolCall, TerminalEntry } from "./types";
import { DiffViewer } from "./DiffViewer";
import { parseDiff, isDiffLike, hunkToRejectPrompt } from "../lib/diffParser";
import type { DiffHunk } from "../lib/diffParser";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { Badge, IconButton, StateDot } from "./ui";
import { ProviderEventGroup } from "./ProviderEventGroup";
import { renderMd } from "../lib/markdown";
import { buildToolTree, type ToolCallGroupItem } from "../lib/toolGroups";
import { formatToolDisplay, isRunningStatus, isSubagentRow } from "../lib/toolDisplay";

export type { ToolCallGroupItem };

/** What every row in one group's tree needs. Shared through context so nested rows do not
 *  re-thread a dozen props per level. */
type ToolTree = {
  live: boolean;
  cwd?: string | null;
  terminals?: Record<string, TerminalEntry>;
  childrenOf: (toolCallId: string) => AppToolCall[];
  permissionFor: (toolCallId: string) => AppPermission | null;
  /** The row owning the permission, and its ancestors; they render open. */
  opensFor: (toolCallId: string) => boolean;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
  onRejectHunk?: (rejectPrompt: string) => void;
  /** Stops a running sub-agent; rejects when the stop could not be sent. */
  onStopAgent?: (tc: AppToolCall) => void | Promise<void>;
};

const ToolTreeContext = createContext<ToolTree>({
  live: false,
  childrenOf: () => [],
  permissionFor: () => null,
  opensFor: () => false,
});

function ToolGroupThought(props: { text: string }) {
  return (
    <details class="agent-thought-block">
      <summary class="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left">
        <Badge tone="neutral" variant="subtle">Thought</Badge>
      </summary>
      <div class="border-t theme-border px-2.5 py-2 text-[11px] theme-muted font-mono leading-relaxed whitespace-pre-wrap break-words max-h-72 overflow-auto">
        {props.text}
      </div>
    </details>
  );
}

function ToolGroupExecutionText(props: { text: string }) {
  return <div class="tool-execution-text md-prose">{renderMd(props.text)}</div>;
}

/** A finished message can still carry a tool the provider never closed (cancelled or dropped
 *  turn); outside a live stream it is shown as unfinished instead of pulsing forever. */
function tcStatusDot(status: string, live = true): string {
  const normalized = status.replace(/[-_]/g, "").toLowerCase();
  if (normalized === "success" || normalized === "completed") return "ui-tool-status-success";
  if (normalized === "failure" || normalized === "failed" || normalized === "error" || normalized === "declined" || normalized === "cancelled" || normalized === "canceled") return "ui-tool-status-danger";
  if (normalized === "interrupted") return "ui-tool-status-idle";
  return live ? "ui-tool-status-warning animate-pulse" : "ui-tool-status-idle";
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

type ToolCallItemProps = {
  tc: AppToolCall;
  nested?: boolean;
};

/** Pretty JSON for display. Computed on demand: tool payloads can be large, and the stored
 *  message carries only the raw values (the `*Json` mirrors are a live-only legacy). */
const prettyJson = (value: unknown): string | undefined => {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "string") return value;
  try { return JSON.stringify(value, null, 2); } catch { return String(value); }
};

/** The patch a file-editing tool reported, read straight from its raw output. */
const diffTextOf = (tc: AppToolCall): string | null => {
  let raw: unknown = tc.rawOutput;
  if (raw === undefined && tc.rawOutputJson) {
    try { raw = JSON.parse(tc.rawOutputJson); } catch { raw = tc.rawOutputJson; }
  }
  const obj = raw as { patch?: unknown; diff?: unknown } | null;
  const text = typeof raw === "string" ? raw
    : typeof obj?.patch === "string" ? obj.patch
    : typeof obj?.diff === "string" ? obj.diff
    : null;
  return text && isDiffLike(text) ? text : null;
};

function ToolCallItem(props: ToolCallItemProps) {
  const tree = useContext(ToolTreeContext);
  const tc = () => props.tc;
  const hasRawInput = () => tc().rawInput != null || !!tc().rawInputJson;
  const hasRawOutput = () => tc().rawOutput != null || !!tc().rawOutputJson;
  const hasContent = () => (tc().content?.length ?? 0) > 0 || !!tc().contentJson;
  const diffText = () => diffTextOf(tc());
  // The debug payload is pretty-printed only while it is open.
  const [debugOpen, setDebugOpen] = createSignal(false);
  const terminalEntry = () => {
    const tid = terminalIdOf(tc());
    if (!tid) return null;
    return tree.terminals?.[tid] ?? null;
  };
  const inlinePermission = () => tree.permissionFor(tc().toolCallId);
  const showPermission = () => !!inlinePermission();
  const children = () => tree.childrenOf(tc().toolCallId);
  const isAgent = () => displayInfo().category === "task";
  const running = () => isRunningStatus(tc().status);
  const canStop = () => !!tree.onStopAgent && tree.live && isSubagentRow(tc()) && running();
  // Disabled from the click until the row's status moves on, so a slow stop is not re-sent.
  const [stopping, setStopping] = createSignal(false);
  createEffect(() => { tc().status; setStopping(false); });
  const stop = async () => {
    setStopping(true);
    try {
      await tree.onStopAgent?.(tc());
    } catch {
      setStopping(false);
    }
  };
  // Rows that need a decision, and the sub-agents holding them, open on their own; rows
  // running a sub-agent open while it works. Never closes a row the user opened.
  let detailsEl: HTMLDetailsElement | undefined;
  const autoOpen = () =>
    tree.opensFor(tc().toolCallId) || (isAgent() && tree.live && running());
  createEffect(() => { if (autoOpen() && detailsEl) detailsEl.open = true; });
  /** The task a sub-agent was given, as its parent sent it. */
  const agentPrompt = () => {
    const input = tc().rawInput as { prompt?: unknown } | null | undefined;
    return isAgent() && typeof input?.prompt === "string" ? input.prompt : null;
  };

  const displayInfo = createMemo(() => formatToolDisplay(tc(), tree.cwd));

  return (
    <details
      ref={detailsEl}
      class="tool-call-item group/tc"
      classList={{ "needs-approval": showPermission() }}
    >
      <summary class="tool-call-item-summary select-none">
        <span class={`ui-tool-status-dot shrink-0 ${tcStatusDot(tc().status, tree.live)}`} />
        <span class="tool-call-item-kind" data-category={displayInfo().category}>
          {displayInfo().categoryBadge}
        </span>

        {/* Concise target summary (file path / command / query), plus the exact
            command as a secondary line when the provider also sent a human
            description (e.g. Claude's Bash `description` field). */}
        <span class="min-w-0 flex-1 flex flex-col justify-center leading-tight">
          <span
            class="tool-call-item-target truncate"
            title={displayInfo().targetSummary || tc().title || tc().toolCallId}
          >
            {displayInfo().targetSummary || tc().title || tc().toolCallId}
          </span>
          <Show when={displayInfo().secondaryInfo}>
            <span class="tool-call-item-secondary truncate" title={displayInfo().secondaryInfo}>
              {displayInfo().secondaryInfo}
            </span>
          </Show>
        </span>

        <Show when={displayInfo().background}>
          <Badge tone="info">bg</Badge>
        </Show>
        <Show when={tc().parentId && !props.nested}>
          <Badge tone="info">subagent</Badge>
        </Show>
        <Show when={terminalEntry()}>
          <Badge tone="info">term</Badge>
        </Show>
        <Show when={showPermission()}>
          <Badge tone="warning">approval</Badge>
        </Show>
        <Show when={canStop()}>
          <IconButton
            size="sm"
            aria-label="Stop sub-agent"
            title="Stop sub-agent"
            disabled={stopping()}
            onClick={(e) => { e.preventDefault(); e.stopPropagation(); void stop(); }}
          >
            <Square size={10} fill="currentColor" />
          </IconButton>
        </Show>
      </summary>
      <Show when={showPermission()}>
        <div class="tool-permission-block">
          <PermissionBody
            permission={inlinePermission()!}
            onChoose={(id) => tree.onApprove?.(id)}
            onDeny={() => tree.onDeny?.()}
          />
        </div>
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
              class={`block w-full text-left font-mono break-all hover:text-[var(--ui-accent)] transition-colors ${tree.onFileClick ? "cursor-pointer" : "cursor-default"}`}
              onClick={() => tree.onFileClick?.(loc.path, tc().kind)}
            >
              {loc.path}{loc.line ? `:${loc.line}` : ""}
            </button>
          )}</For>
        </div>
      </Show>
      <Show when={agentPrompt()}>
        {(prompt) => <div class="tool-agent-prompt md-prose">{renderMd(prompt())}</div>}
      </Show>
      <Show when={children().length > 0}>
        <div class="tool-call-children">
          <For each={children()}>{(child) => (
            <ToolCallItem tc={child} nested />
          )}</For>
        </div>
      </Show>
      <Show when={isAgent() && tc().outputLog}>
        {/* Markdown is parsed once the sub-agent is done; re-parsing every chunk of a growing
            reply costs more than it shows. */}
        <Show
          when={!running()}
          fallback={<div class="tool-agent-output whitespace-pre-wrap break-words">{tc().outputLog}</div>}
        >
          <div class="tool-agent-output md-prose">{renderMd(tc().outputLog!)}</div>
        </Show>
      </Show>
      <Show when={!isAgent() && tc().outputLog}>
        <div class="px-2 pb-1.5">
          <div class="text-[9px] uppercase tracking-wide theme-muted">output</div>
          <pre class="mt-0.5 max-h-48 overflow-y-auto whitespace-pre-wrap break-words rounded bg-[var(--ui-surface-muted)] p-2 font-mono text-[10px] theme-muted">{tc().outputLog}</pre>
        </div>
      </Show>
      <Show when={diffText()}>
        {(text) => {
          const diffData = () => parseDiff(text());
          return (
            <Show when={diffData()}>
              {(diffs) => (
                <div class="border-t theme-border bg-[var(--ui-panel-2)] p-2.5">
                  <DiffViewer
                    diffs={diffs()}
                    onRejectHunk={(filePath: string, hunk: DiffHunk) => {
                      tree.onRejectHunk?.(hunkToRejectPrompt(filePath, hunk));
                    }}
                  />
                </div>
              )}
            </Show>
          );
        }}
      </Show>
      {/* Discreet Debug Payload for raw JSON */}
      <Show when={hasRawInput() || (hasRawOutput() && !diffText()) || hasContent()}>
        <details
          class="border-t theme-border bg-[var(--ui-panel-2)]"
          onToggle={(e) => setDebugOpen(e.currentTarget.open)}
        >
          <summary class="px-3 py-1.5 text-[9px] font-mono theme-muted cursor-pointer hover:text-[var(--ui-text)] transition-colors select-none">
            Debug Payload ({tc().toolCallId})
          </summary>
          <Show when={debugOpen()}>
          <div class="p-2 space-y-2 border-t theme-border">
            <Show when={hasRawInput()}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Input:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().rawInputJson ?? prettyJson(tc().rawInput)}</pre>
              </div>
            </Show>
            <Show when={hasRawOutput() && !diffText()}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Output:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().rawOutputJson ?? prettyJson(tc().rawOutput)}</pre>
              </div>
            </Show>
            <Show when={hasContent()}>
              <div>
                <span class="text-[8.5px] font-mono uppercase theme-muted">Content:</span>
                <pre class="whitespace-pre-wrap break-words mt-0.5 p-2 rounded bg-[var(--ui-surface-muted)] text-[10px] font-mono theme-muted max-h-48 overflow-y-auto">{tc().contentJson ?? prettyJson(tc().content)}</pre>
              </div>
            </Show>
          </div>
          </Show>
        </details>
      </Show>
    </details>
  );
}

export function ToolCallGroup(props: {
  tools?: AppToolCall[];
  items?: ToolCallGroupItem[];
  streaming: boolean;
  cwd?: string | null;
  terminals?: Record<string, TerminalEntry>;
  pendingPermission?: AppPermission | null;
  onApprove?: (optionId: string) => void;
  onDeny?: () => void;
  onFileClick?: (path: string, kind: string) => void;
  onRejectHunk?: (rejectPrompt: string) => void;
  onStopAgent?: (tc: AppToolCall) => void | Promise<void>;
}) {
  const groupItems = createMemo<ToolCallGroupItem[]>(() =>
    props.items ?? (props.tools ?? []).map((tool) => ({ kind: "tool", tool })),
  );
  const tools = createMemo(() =>
    groupItems()
      .filter((item): item is { kind: "tool"; tool: AppToolCall } => item.kind === "tool")
      .map((item) => item.tool),
  );
  const hasPendingPermission = () => !!props.pendingPermission;
  const [expanded, setExpanded] = createSignal(false);

  const treeIndex = createMemo(() => buildToolTree(tools(), props.pendingPermission));
  const topItems = createMemo(() =>
    groupItems().filter((item) => item.kind !== "tool" || !treeIndex().nestedIds.has(item.tool.toolCallId)),
  );
  const topTools = createMemo(() => tools().filter((t) => !treeIndex().nestedIds.has(t.toolCallId)));

  createEffect(() => {
    if (hasPendingPermission()) setExpanded(true);
  });

  // Counted over the same top-level rows the title counts; a sub-agent's own calls are
  // summarised by its row.
  const statusCounts = createMemo(() => {
    let success = 0, error = 0;
    const top = topTools();
    for (const t of top) {
      const status = t.status.replace(/[-_]/g, "").toLowerCase();
      if (status === "success" || status === "completed") success++;
      else if (status === "failure" || status === "failed" || status === "error" || status === "declined" || status === "cancelled" || status === "canceled") error++;
    }
    return { success, error, pending: top.length - success - error };
  });

  const tree: ToolTree = {
    get live() { return props.streaming; },
    get cwd() { return props.cwd; },
    get terminals() { return props.terminals; },
    childrenOf: (id) => treeIndex().childMap.get(id) ?? [],
    permissionFor: (id) => (id === treeIndex().ownerId ? props.pendingPermission ?? null : null),
    opensFor: (id) => treeIndex().openIds.has(id),
    get onApprove() { return props.onApprove; },
    get onDeny() { return props.onDeny; },
    get onFileClick() { return props.onFileClick; },
    get onRejectHunk() { return props.onRejectHunk; },
    get onStopAgent() { return props.onStopAgent; },
  };

  // Title says what happened; detail is secondary context. Exceptions (approval, failures) get
  // a badge on the right; a clean run needs no badge at all.
  const groupSummary = createMemo((): { title: string; detail?: string } => {
    const top = topTools();
    if (top.length === 0) return { title: "Execution" };
    if (top.length === 1) {
      const display = formatToolDisplay(top[0], props.cwd);
      return { title: display.actionLabel, detail: display.targetSummary };
    }
    const categories = Array.from(
      new Set(top.map((t) => formatToolDisplay(t, props.cwd).categoryBadge)),
    );
    return { title: `Ran ${top.length} tools`, detail: categories.join(", ") };
  });

  return (
    <div class="tool-call-card" classList={{ "needs-approval": hasPendingPermission(), "is-open": expanded() }}>
      <button
        class="tool-call-summary select-none"
        aria-expanded={expanded()}
        onClick={() => setExpanded(v => !v)}
      >
        {/* Status at rest; on hover the slot cross-fades to the disclosure chevron. */}
        <span class="tool-call-summary-icon" aria-hidden="true">
          <span class="tool-call-summary-status">
            <Switch fallback={<Check size={13} stroke-width={2.25} />}>
              <Match when={hasPendingPermission()}><StateDot state="warning" size={10} /></Match>
              <Match when={props.streaming && statusCounts().pending > 0}><StateDot state="ongoing" size={11} /></Match>
              <Match when={statusCounts().error > 0}><StateDot state="error" size={10} /></Match>
            </Switch>
          </span>
          <ChevronDown size={13} class="tool-call-summary-chevron" />
        </span>
        <span class="tool-call-summary-label">
          <span class="tool-call-summary-title">{groupSummary().title}</span>
          <Show when={groupSummary().detail}>
            <span class="tool-call-summary-detail">{groupSummary().detail}</span>
          </Show>
        </span>
        <span class="flex items-center gap-1.5 ml-auto shrink-0">
          <Show when={hasPendingPermission()}>
            <Badge tone="warning">Approval needed</Badge>
          </Show>
          <Show when={statusCounts().error > 0}>
            <Badge tone="danger">{statusCounts().error} failed</Badge>
          </Show>
          <Show when={statusCounts().pending > 0 && !props.streaming && !hasPendingPermission()}>
            <Badge tone="neutral" title="These calls never reported a result">{statusCounts().pending} unfinished</Badge>
          </Show>
        </span>
      </button>
      <Show when={expanded()}>
        <ToolTreeContext.Provider value={tree}>
        <div class="tool-call-list">
          <For each={topItems()}>{(item) => {
            return item.kind === "tool" ? (
              <ToolCallItem tc={item.tool} />
            ) : item.kind === "events" ? (
              <ProviderEventGroup events={item.events} />
            ) : item.kind === "thought" ? (
              <ToolGroupThought text={item.text} />
            ) : (
              <ToolGroupExecutionText text={item.text} />
            );
          }}</For>
        </div>
        </ToolTreeContext.Provider>
      </Show>
    </div>
  );
}
