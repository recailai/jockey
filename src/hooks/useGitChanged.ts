import { onCleanup, onMount } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Handler = (args: { absPath: string; relPath: string }) => void;

export function useGitChanged(cwd: () => string | null | undefined, onChange: Handler) {
  let unlisten: UnlistenFn | null = null;
  onMount(async () => {
    unlisten = await listen<{ path: string }>("git/changed", (ev) => {
      const base = cwd() ?? "";
      const abs = ev.payload?.path ?? "";
      if (!base || !abs.startsWith(base)) return;
      const rel = abs.slice(base.length).replace(/^\/+/, "");
      onChange({ absPath: abs, relPath: rel });
    });
  });
  onCleanup(() => {
    if (unlisten) unlisten();
  });
}
