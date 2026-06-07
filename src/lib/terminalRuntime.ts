import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { terminalApi } from "./tauriApi";

export type SessionTerminalEntry = {
  sessionId: string;
  terminalId: string | null;
  subtitle: string;
  mountEl: HTMLDivElement;
  term: Terminal;
  fitAddon: FitAddon;
  needsFreshPty: boolean;
};

type SubtitleListener = (sessionId: string, subtitle: string) => void;

const bySession = new Map<string, SessionTerminalEntry>();
const startPromises = new Map<string, Promise<SessionTerminalEntry>>();
const subtitleListeners = new Set<SubtitleListener>();
let listenersReady: Promise<void> | null = null;
let unlisteners: UnlistenFn[] = [];

function themeFromCss(): Terminal["options"]["theme"] {
  const styles = getComputedStyle(document.documentElement);
  const getVar = (name: string) => styles.getPropertyValue(name).trim();
  return {
    background: getVar("--ui-terminal-bg") || "#ffffff",
    foreground: getVar("--ui-terminal-text") || "#1f1f1f",
    cursor: getVar("--ui-terminal-text") || "#1f1f1f",
    selectionBackground: getVar("--ui-terminal-selection") || "#d9d9dc",
    black: getVar("--ui-terminal-black"),
    red: getVar("--ui-terminal-red"),
    green: getVar("--ui-terminal-green"),
    yellow: getVar("--ui-terminal-yellow"),
    blue: getVar("--ui-terminal-blue"),
    magenta: getVar("--ui-terminal-magenta"),
    cyan: getVar("--ui-terminal-cyan"),
    white: getVar("--ui-terminal-white"),
    brightBlack: getVar("--ui-terminal-brightBlack"),
    brightRed: getVar("--ui-terminal-brightRed"),
    brightGreen: getVar("--ui-terminal-brightGreen"),
    brightYellow: getVar("--ui-terminal-brightYellow"),
    brightBlue: getVar("--ui-terminal-brightBlue"),
    brightMagenta: getVar("--ui-terminal-brightMagenta"),
    brightCyan: getVar("--ui-terminal-brightCyan"),
    brightWhite: getVar("--ui-terminal-brightWhite"),
  };
}

export function updateTerminalThemes(): void {
  const newTheme = themeFromCss();
  for (const entry of bySession.values()) {
    entry.term.options.theme = newTheme;
  }
}

function setSubtitle(sessionId: string, subtitle: string): void {
  const entry = bySession.get(sessionId);
  if (entry) entry.subtitle = subtitle;
  for (const fn of subtitleListeners) fn(sessionId, subtitle);
}

export function onTerminalSubtitle(listener: SubtitleListener): () => void {
  subtitleListeners.add(listener);
  return () => subtitleListeners.delete(listener);
}

function createEntry(sessionId: string): SessionTerminalEntry {
  const mountEl = document.createElement("div");
  mountEl.className = "terminal-host h-full w-full overflow-hidden";
  const term = new Terminal({
    disableStdin: false,
    cursorBlink: true,
    cursorStyle: "bar",
    cursorWidth: 2,
    scrollback: 5000,
    convertEol: true,
    drawBoldTextInBrightColors: true,
    theme: themeFromCss(),
    fontSize: 12,
    lineHeight: 1.35,
    letterSpacing: 0,
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace",
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(mountEl);
  term.onData((data) => {
    const entry = bySession.get(sessionId);
    const id = entry?.terminalId;
    if (!id) return;
    void terminalApi.write(id, data).catch(() => {});
  });
  const entry: SessionTerminalEntry = {
    sessionId,
    terminalId: null,
    subtitle: "Starting...",
    mountEl,
    term,
    fitAddon,
    needsFreshPty: true,
  };
  bySession.set(sessionId, entry);
  return entry;
}

export function getSessionTerminal(sessionId: string): SessionTerminalEntry {
  return bySession.get(sessionId) ?? createEntry(sessionId);
}

export function mountSessionTerminal(sessionId: string, host: HTMLDivElement): SessionTerminalEntry {
  const entry = getSessionTerminal(sessionId);
  const wasDetached = !entry.mountEl.isConnected;
  if (entry.mountEl.parentElement !== host) {
    host.replaceChildren(entry.mountEl);
  }
  if (wasDetached) {
    entry.needsFreshPty = true;
    entry.terminalId = null;
  }
  return entry;
}

export function ensureTerminalListeners(): Promise<void> {
  if (listenersReady) return listenersReady;
  listenersReady = Promise.all([
    listen<{ terminalId: string; appSessionId: string; data: string }>(
      "terminal/session_output",
      (ev) => {
        const entry = bySession.get(ev.payload.appSessionId);
        if (!entry || entry.terminalId !== ev.payload.terminalId) return;
        entry.term.write(ev.payload.data);
      },
    ),
    listen<{ terminalId: string; appSessionId: string; exitCode?: number | null }>(
      "terminal/session_exit",
      (ev) => {
        const entry = bySession.get(ev.payload.appSessionId);
        if (!entry || entry.terminalId !== ev.payload.terminalId) return;
        const code = ev.payload.exitCode;
        entry.term.writeln(`\r\n[process exited${typeof code === "number" ? ` with code ${code}` : ""}]`);
        entry.terminalId = null;
        entry.needsFreshPty = true;
        setSubtitle(ev.payload.appSessionId, "Terminal exited");
      },
    ),
  ]).then((fns) => {
    unlisteners = fns;
  });
  return listenersReady;
}

async function startSessionTerminal(
  sessionId: string,
  opts?: { forceNew?: boolean },
): Promise<SessionTerminalEntry> {
  const entry = getSessionTerminal(sessionId);
  const forceNew = (opts?.forceNew ?? false) || entry.needsFreshPty;

  if (forceNew && entry.terminalId) {
    await terminalApi.stop(entry.terminalId).catch(() => {});
    entry.terminalId = null;
  }
  if (entry.terminalId && !forceNew) {
    entry.needsFreshPty = false;
    return entry;
  }

  setSubtitle(sessionId, "Starting...");
  try {
    const started = await terminalApi.start(sessionId, forceNew);
    entry.terminalId = started.terminalId;
    entry.needsFreshPty = false;
    setSubtitle(
      sessionId,
      `${started.cwd} - ${started.shell.split("/").pop() ?? started.shell}`,
    );
    fitSessionTerminal(sessionId);
    return entry;
  } catch (e) {
    entry.terminalId = null;
    entry.needsFreshPty = true;
    setSubtitle(sessionId, "Terminal failed");
    entry.term.writeln(`\r\nTerminal failed: ${String(e)}\r\n`);
    throw e;
  }
}

export async function ensureSessionTerminal(
  sessionId: string,
  opts?: { forceNew?: boolean },
): Promise<SessionTerminalEntry> {
  await ensureTerminalListeners();
  const forceNew = opts?.forceNew ?? false;
  const entry = getSessionTerminal(sessionId);
  if (entry.terminalId && !forceNew && !entry.needsFreshPty) return entry;

  if (forceNew) {
    startPromises.delete(sessionId);
  }

  const pending = startPromises.get(sessionId);
  if (pending && !forceNew) return pending;

  const promise = startSessionTerminal(sessionId, opts);
  startPromises.set(sessionId, promise);
  try {
    return await promise;
  } finally {
    if (startPromises.get(sessionId) === promise) {
      startPromises.delete(sessionId);
    }
  }
}

export function fitSessionTerminal(sessionId: string): void {
  const entry = bySession.get(sessionId);
  if (!entry) return;
  const fit = () => {
    try {
      entry.fitAddon.fit();
      const cols = entry.term.cols;
      const rows = entry.term.rows;
      if (entry.terminalId && cols > 0 && rows > 0) {
        void terminalApi.resize(entry.terminalId, cols, rows).catch(() => {});
      }
    } catch {
      // Panel may still be hidden or zero-sized during layout.
    }
  };
  requestAnimationFrame(() => {
    fit();
    requestAnimationFrame(fit);
  });
}

export function writeSessionCommand(sessionId: string, command: string): void {
  const entry = bySession.get(sessionId);
  if (!entry?.terminalId || !command.trim()) return;
  void terminalApi.write(entry.terminalId, `${command.trimEnd()}\r`).catch(() => {});
  entry.term.focus();
}

export async function destroySessionTerminal(sessionId: string): Promise<void> {
  startPromises.delete(sessionId);
  const entry = bySession.get(sessionId);
  if (!entry) return;
  if (entry.terminalId) {
    await terminalApi.stop(entry.terminalId).catch(() => {});
  }
  entry.term.dispose();
  bySession.delete(sessionId);
}

export function disposeTerminalRuntime(): void {
  for (const id of [...bySession.keys()]) {
    void destroySessionTerminal(id);
  }
  for (const unlisten of unlisteners) unlisten();
  unlisteners = [];
  listenersReady = null;
}
