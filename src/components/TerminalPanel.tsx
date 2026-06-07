import { Show, createEffect, createSignal, onMount } from "solid-js";
import type { Accessor } from "solid-js";
import { RefreshCw, Terminal as TerminalIcon, X } from "lucide-solid";
import type { AppSession } from "./types";
import {
  ensureSessionTerminal,
  fitSessionTerminal,
  getSessionTerminal,
  mountSessionTerminal,
  onTerminalSubtitle,
  writeSessionCommand,
} from "../lib/terminalRuntime";
import {
  DockPanel,
  DockPanelToolbar,
  DockPanelToolbarActions,
  DockPanelToolbarTitle,
  PanelHeader,
  PanelHeaderAction,
} from "./ui";

type TerminalPanelProps = {
  activeSession: Accessor<AppSession | null>;
  visible: boolean;
  onClose: () => void;
  commandRequest?: { id: number; command: string } | null;
  dockEmbedded?: boolean;
};

export default function TerminalPanel(props: TerminalPanelProps) {
  const [subtitle, setSubtitle] = createSignal("Starting...");
  let hostEl: HTMLDivElement | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let lastCommandRequestId = -1;

  const sessionId = () => props.activeSession()?.id ?? null;

  const syncVisibleTerminal = async (sid: string) => {
    if (!props.visible || !hostEl) return;
    mountSessionTerminal(sid, hostEl);
    try {
      const entry = await ensureSessionTerminal(sid);
      setSubtitle(entry.subtitle);
      fitSessionTerminal(sid);
      entry.term.focus();
    } catch {
      setSubtitle(getSessionTerminal(sid).subtitle);
    }
  };

  const bindHost = (el: HTMLDivElement | undefined) => {
    if (!el) {
      hostEl = undefined;
      resizeObserver?.disconnect();
      return;
    }
    hostEl = el;
    resizeObserver?.disconnect();
    resizeObserver = new ResizeObserver(() => {
      const sid = sessionId();
      if (sid && props.visible) fitSessionTerminal(sid);
    });
    resizeObserver.observe(el);
    const sid = sessionId();
    if (sid && props.visible) void syncVisibleTerminal(sid);
  };

  onMount(() => {
    const offSubtitle = onTerminalSubtitle((sid, text) => {
      if (sid === sessionId()) setSubtitle(text);
    });
    return () => {
      resizeObserver?.disconnect();
      offSubtitle();
      hostEl = undefined;
    };
  });

  createEffect(() => {
    const sid = sessionId();
    const visible = props.visible;
    if (!sid) {
      setSubtitle("No active AppSession");
      return;
    }
    setSubtitle(getSessionTerminal(sid).subtitle);
    if (!visible) return;
    void syncVisibleTerminal(sid);
  });

  createEffect(() => {
    const req = props.commandRequest;
    const sid = sessionId();
    const visible = props.visible;
    if (!req || !sid || !visible) return;
    if (lastCommandRequestId === req.id) return;
    lastCommandRequestId = req.id;

    void (async () => {
      if (hostEl) mountSessionTerminal(sid, hostEl);
      try {
        const entry = await ensureSessionTerminal(sid);
        if (props.commandRequest?.id !== req.id) return;
        setSubtitle(entry.subtitle);
        const command = req.command.trimEnd();
        if (!command) {
          fitSessionTerminal(sid);
          entry.term.focus();
          return;
        }
        writeSessionCommand(sid, command);
        fitSessionTerminal(sid);
      } catch {
        setSubtitle(getSessionTerminal(sid).subtitle);
      }
    })();
  });

  const restart = () => {
    const sid = sessionId();
    if (!sid) return;
    lastCommandRequestId = -1;
    void ensureSessionTerminal(sid, { forceNew: true }).then((entry) => {
      if (hostEl) mountSessionTerminal(sid, hostEl);
      setSubtitle(entry.subtitle);
      fitSessionTerminal(sid);
      entry.term.focus();
    });
  };

  const headerActions = (
    <DockPanelToolbarActions>
      <PanelHeaderAction onClick={restart} title="Restart terminal">
        <RefreshCw size={13} />
      </PanelHeaderAction>
      <Show when={!props.dockEmbedded}>
        <PanelHeaderAction onClick={props.onClose} title="Close terminal">
          <X size={13} />
        </PanelHeaderAction>
      </Show>
    </DockPanelToolbarActions>
  );

  return (
    <DockPanel
      class={`terminal-panel tool-dock-terminal-layer${props.visible ? " is-visible" : ""}`}
    >
      <Show when={props.dockEmbedded} fallback={
        <PanelHeader class="panel-header terminal-header">
          <div class="flex min-w-0 items-center gap-2">
            <TerminalIcon size={15} class="theme-muted" />
            <div class="min-w-0">
              <div class="text-[12px] font-medium theme-text">Terminal</div>
              <div class="truncate text-[10.5px] theme-muted">{subtitle()}</div>
            </div>
          </div>
          <div class="ml-auto flex items-center gap-1">{headerActions}</div>
        </PanelHeader>
      }>
        <DockPanelToolbar>
          <TerminalIcon size={14} class="theme-muted shrink-0" />
          <DockPanelToolbarTitle class="truncate">{subtitle()}</DockPanelToolbarTitle>
          {headerActions}
        </DockPanelToolbar>
      </Show>
      <div class="terminal-surface">
        <div ref={bindHost} class="terminal-host-shell h-full w-full overflow-hidden" />
      </div>
    </DockPanel>
  );
}
