import { invoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

export type { UnlistenFn };

/**
 * How the typed API reaches the engine and how engine events come back. The desktop speaks
 * Tauri IPC; the phone companion installs an HTTP + SSE transport that maps the same command
 * names and event names onto the web gateway, so every component and reducer above this
 * seam is shared.
 */
export type Transport = {
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, handler: (ev: { payload: T }) => void): Promise<UnlistenFn>;
  /** Open the external URL the way the host platform would. */
  openExternal(url: string): void;
};

const tauriTransport: Transport = {
  call: (command, args) => invoke(command, args),
  listen: (event, handler) => tauriListen(event, handler),
  openExternal: (url) => {
    void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(url));
  },
};

let active: Transport = tauriTransport;

export function setTransport(next: Transport) {
  active = next;
}

export const call = <T>(command: string, args?: Record<string, unknown>) => active.call<T>(command, args);

export const listen = <T>(event: string, handler: (ev: { payload: T }) => void) =>
  active.listen<T>(event, handler);

export const openExternal = (url: string) => active.openExternal(url);
