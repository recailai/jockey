import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { RefreshCw, Terminal as TerminalIcon, X } from "lucide-solid";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { AppSession } from "./types";
import { terminalApi } from "../lib/tauriApi";
import { Panel, PanelHeader, PanelHeaderAction } from "./ui";

type TerminalPanelProps = {
  session: AppSession | null;
  onClose: () => void;
  commandRequest?: { id: number; command: string } | null;
};

type TerminalOutputEvent = {
  terminalId: string;
  appSessionId: string;
  data: string;
};

type TerminalExitEvent = {
  terminalId: string;
  appSessionId: string;
  exitCode?: number | null;
};

export default function TerminalPanel(props: TerminalPanelProps) {
  const [terminalId, setTerminalId] = createSignal<string | null>(null);
  const [subtitle, setSubtitle] = createSignal("Starting...");
  const [error, setError] = createSignal<string | null>(null);
  const [starting, setStarting] = createSignal(false);

  let containerEl: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let unlisteners: UnlistenFn[] = [];
  let ownedTerminalId: string | null = null;
  let activeSessionId: string | null = null;
  let startingForSession: string | null = null;
  let handledCommandRequestId: number | null = null;

  const stopOwnedTerminal = () => {
    const id = ownedTerminalId;
    ownedTerminalId = null;
    activeSessionId = null;
    setTerminalId(null);
    if (id) void terminalApi.stop(id).catch(() => {});
  };

  const startTerminal = async (session: AppSession | null) => {
    if (!session?.id) {
      setSubtitle("No active AppSession");
      return;
    }
    if (starting() || startingForSession === session.id) return;
    stopOwnedTerminal();
    setError(null);
    setStarting(true);
    startingForSession = session.id;
    term?.reset();
    term?.writeln("Starting terminal...");
    try {
      const started = await terminalApi.start(session.id);
      ownedTerminalId = started.terminalId;
      activeSessionId = session.id;
      setTerminalId(started.terminalId);
      setSubtitle(`${started.cwd} - ${started.shell.split("/").pop() ?? started.shell}`);
      term?.reset();
      term?.focus();
    } catch (e) {
      setError(String(e));
      setSubtitle("Terminal failed");
      term?.writeln(`Terminal failed: ${String(e)}`);
    } finally {
      setStarting(false);
      startingForSession = null;
    }
  };

  onMount(() => {
    if (!containerEl) return;
    const styles = getComputedStyle(document.documentElement);
    term = new Terminal({
      disableStdin: false,
      cursorBlink: true,
      scrollback: 5000,
      convertEol: true,
      theme: {
        background: styles.getPropertyValue("--ui-terminal-bg").trim() || "#ffffff",
        foreground: styles.getPropertyValue("--ui-terminal-text").trim() || "#1f1f1f",
        cursor: styles.getPropertyValue("--ui-terminal-text").trim() || "#1f1f1f",
        selectionBackground: "#d9d9dc",
      },
      fontSize: 12,
      lineHeight: 1.35,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerEl);
    fitAddon.fit();
    term.onData((data) => {
      const id = terminalId();
      if (!id) return;
      void terminalApi.write(id, data).catch((e) => setError(String(e)));
    });
    resizeObserver = new ResizeObserver(() => fitAddon?.fit());
    resizeObserver.observe(containerEl);

    void Promise.all([
      listen<TerminalOutputEvent>("terminal/session_output", (ev) => {
        if (ev.payload.terminalId !== ownedTerminalId) return;
        term?.write(ev.payload.data);
      }),
      listen<TerminalExitEvent>("terminal/session_exit", (ev) => {
        if (ev.payload.terminalId !== ownedTerminalId) return;
        const code = ev.payload.exitCode;
        term?.writeln(`\r\n[process exited${typeof code === "number" ? ` with code ${code}` : ""}]`);
        setSubtitle("Terminal exited");
        ownedTerminalId = null;
        activeSessionId = null;
        setTerminalId(null);
      }),
    ]).then((fns) => {
      unlisteners = fns;
      void startTerminal(props.session);
    });
  });

  createEffect(() => {
    const sid = props.session?.id ?? null;
    if (!term || !sid) return;
    if (startingForSession === sid) return;
    if ((!ownedTerminalId || activeSessionId !== sid) && !starting()) {
      void startTerminal(props.session);
    }
  });

  createEffect(() => {
    const req = props.commandRequest;
    const id = terminalId();
    if (!req || !id || handledCommandRequestId === req.id) return;
    handledCommandRequestId = req.id;
    const command = req.command.trimEnd();
    if (!command) return;
    void terminalApi.write(id, `${command}\r`).catch((e) => setError(String(e)));
    term?.focus();
  });

  onCleanup(() => {
    stopOwnedTerminal();
    resizeObserver?.disconnect();
    for (const unlisten of unlisteners) unlisten();
    term?.dispose();
  });

  return (
    <Panel class="tool-panel terminal-panel">
      <PanelHeader class="panel-header terminal-header">
        <div class="flex min-w-0 items-center gap-2">
          <TerminalIcon size={15} class="theme-muted" />
          <div class="min-w-0">
            <div class="text-[12px] font-medium theme-text">Terminal</div>
            <div class="truncate text-[10.5px] theme-muted">{subtitle()}</div>
          </div>
        </div>
        <div class="ml-auto flex items-center gap-1">
          <PanelHeaderAction
            onClick={() => void startTerminal(props.session)}
            title="Restart terminal"
          >
            <RefreshCw size={14} />
          </PanelHeaderAction>
          <PanelHeaderAction onClick={props.onClose} title="Close terminal">
            <X size={14} />
          </PanelHeaderAction>
        </div>
      </PanelHeader>
      <Show when={error()}>
        <div class="border-b border-[var(--ui-border)] px-3 py-2 text-[11px] text-[var(--ui-state-danger-text)]">
          {error()}
        </div>
      </Show>
      <div class="terminal-surface">
        <div ref={containerEl} class="h-full w-full overflow-hidden px-1 py-0.5" />
      </div>
    </Panel>
  );
}
