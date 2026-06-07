import { onMount, onCleanup } from "solid-js";

export type WorkspaceToolPanel = "git" | "files" | "terminal";

function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (el.isContentEditable) return true;
  return !!el.closest("[contenteditable='true']");
}

export function useKeyboardShortcuts(handlers: {
  newSession: () => void;
  openSettings: () => void;
  toggleManagement: () => void;
  toggleRightDock: () => void;
  openWorkspacePanel: (p: WorkspaceToolPanel) => void;
}): void {
  onMount(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const inEditable = isEditableTarget(e.target);
      switch (e.key) {
        case "k":
          if (inEditable) return;
          e.preventDefault();
          handlers.newSession();
          return;
        case ",":
          e.preventDefault();
          handlers.openSettings();
          return;
        case "m":
          if (e.shiftKey) {
            e.preventDefault();
            handlers.toggleManagement();
          }
          return;
        case "g":
        case "2":
          if (inEditable) return;
          e.preventDefault();
          handlers.openWorkspacePanel("git");
          return;
        case "1":
          if (inEditable) return;
          e.preventDefault();
          handlers.openWorkspacePanel("files");
          return;
        case "3":
          if (inEditable) return;
          e.preventDefault();
          handlers.openWorkspacePanel("terminal");
          return;
        case "b":
          if (!e.shiftKey && !e.altKey && !inEditable) {
            e.preventDefault();
            handlers.toggleRightDock();
          }
          return;
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);
    onCleanup(() => window.removeEventListener("keydown", handleGlobalKeyDown));
  });
}
