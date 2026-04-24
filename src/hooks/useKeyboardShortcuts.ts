import { onMount, onCleanup } from "solid-js";
import type { ActivityPanel } from "../components/ActivityBar";

export function useKeyboardShortcuts(handlers: {
  newSession: () => void;
  toggleManagement: () => void;
  toggleSidebarRestore: () => void;
  setSidebarPanel: (p: ActivityPanel | null) => void;
  cancelCurrentRun: () => void;
}): void {
  onMount(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const tag = (e.target as HTMLElement | null)?.tagName;
      const inEditableTarget = tag === "INPUT" || tag === "TEXTAREA";
      switch (e.key) {
        case "k":
          e.preventDefault(); handlers.newSession(); return;
        case "m":
          if (e.shiftKey) { e.preventDefault(); handlers.toggleManagement(); }
          return;
        case "g":
          e.preventDefault(); handlers.setSidebarPanel("git"); return;
        case "1":
          if (!inEditableTarget) { e.preventDefault(); handlers.setSidebarPanel("git"); }
          return;
        case "2":
          if (!inEditableTarget) { e.preventDefault(); handlers.setSidebarPanel("files"); }
          return;
        case "b":
          if (!e.shiftKey && !e.altKey) { e.preventDefault(); handlers.toggleSidebarRestore(); }
          return;
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);
    onCleanup(() => window.removeEventListener("keydown", handleGlobalKeyDown));
  });
}
