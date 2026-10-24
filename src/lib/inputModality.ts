/**
 * Marks `<html data-input-modality>` as `pointer` or `keyboard` so focus rings show for
 * keyboard navigation but not after a mouse click (design-system.css).
 */
export function trackInputModality(): void {
  const root = document.documentElement;
  const set = (mode: "pointer" | "keyboard") => {
    if (root.dataset.inputModality !== mode) root.dataset.inputModality = mode;
  };
  set("pointer");
  window.addEventListener("pointerdown", () => set("pointer"), { capture: true, passive: true });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Tab" || e.key.startsWith("Arrow") || e.key === "Enter" || e.key === " ") set("keyboard");
  }, { capture: true, passive: true });
}
