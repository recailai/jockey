/** Layout sizing tokens — single source for dock widths, preview ratio, and constraints. */

export const LAYOUT_STORAGE = {
  /** Last selected left-dock tab (panel open state is not restored on launch). */
  leftPanelLast: "jockey:tool.panel.last",
  /** @deprecated migrated to leftPanelLast; no longer restores open state */
  leftPanel: "jockey:tool.panel",
  leftDockWidth: "jockey:leftDockWidth",
  /** @deprecated migrated to leftDockWidth */
  legacyRightToolWidth: "jockey:rightToolPanelWidth",
  rightDockWidth: "jockey:rightDockWidth",
  rightDockOpen: "jockey:rightDockOpen",
  rightPanel: "jockey:rightPanel",
  previewRatio: "jockey:editorChatRatio",
} as const;

export const LEFT_DOCK = {
  defaultWidth: 360,
  minWidth: 280,
  maxWidth: 520,
} as const;

export const RIGHT_DOCK = {
  defaultWidth: 340,
  minWidth: 260,
  maxWidth: 480,
} as const;

export const PREVIEW = {
  defaultRatio: 0.5,
  minRatio: 0.2,
  maxRatio: 0.75,
} as const;

export const CHAT = {
  minWidth: 420,
} as const;

export const COMPOSER = {
  emptyMinHeight: 128,
  activeMaxHeight: 160,
  emptyMaxWidth: 720,
  activeMaxWidth: 980,
} as const;

export const SESSION = {
  chipMaxWidth: 168,
  topbarHeight: 48,
} as const;

export type LeftDockPanel = "files" | "git" | "terminal" | "commit";
export type RightDockPanel = "files" | "git" | "terminal";

export function readStoredWidth(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  try {
    const raw = window.localStorage.getItem(key);
    const n = raw ? parseFloat(raw) : NaN;
    if (Number.isFinite(n)) return Math.min(max, Math.max(min, n));
  } catch { /* ignore */ }
  return fallback;
}

export function readStoredRatio(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  try {
    const raw = window.localStorage.getItem(key);
    const n = raw ? parseFloat(raw) : NaN;
    if (Number.isFinite(n)) return Math.min(max, Math.max(min, n));
  } catch { /* ignore */ }
  return fallback;
}

export function initialLeftDockWidth(): number {
  const primary = readStoredWidth(
    LAYOUT_STORAGE.leftDockWidth,
    NaN,
    LEFT_DOCK.minWidth,
    LEFT_DOCK.maxWidth,
  );
  if (Number.isFinite(primary)) return primary;
  return readStoredWidth(
    LAYOUT_STORAGE.legacyRightToolWidth,
    LEFT_DOCK.defaultWidth,
    LEFT_DOCK.minWidth,
    LEFT_DOCK.maxWidth,
  );
}

export function initialRightDockWidth(): number {
  return readStoredWidth(
    LAYOUT_STORAGE.rightDockWidth,
    RIGHT_DOCK.defaultWidth,
    RIGHT_DOCK.minWidth,
    RIGHT_DOCK.maxWidth,
  );
}

export function initialPreviewRatio(): number {
  return readStoredRatio(
    LAYOUT_STORAGE.previewRatio,
    PREVIEW.defaultRatio,
    PREVIEW.minRatio,
    PREVIEW.maxRatio,
  );
}

/** Last left-dock tab — used when user toggles the dock open, not on cold start. */
export function initialLastLeftPanel(): LeftDockPanel {
  try {
    const raw =
      window.localStorage.getItem(LAYOUT_STORAGE.leftPanelLast)
      ?? window.localStorage.getItem(LAYOUT_STORAGE.leftPanel);
    if (raw === "git" || raw === "commit") return raw === "commit" ? "commit" : "git";
    if (raw === "files" || raw === "terminal") return raw;
  } catch { /* ignore */ }
  return "files";
}

/** @deprecated Left dock starts closed; use initialLastLeftPanel when opening. */
export function initialLeftPanel(): LeftDockPanel | null {
  return null;
}

export function initialRightDockOpen(): boolean {
  try {
    return window.localStorage.getItem(LAYOUT_STORAGE.rightDockOpen) === "1";
  } catch { /* ignore */ }
  return false;
}

export function initialRightPanel(): RightDockPanel | null {
  try {
    const raw = window.localStorage.getItem(LAYOUT_STORAGE.rightPanel);
    if (raw === "files" || raw === "git" || raw === "terminal") return raw;
  } catch { /* ignore */ }
  return null;
}

export function clampDockWidthForMain(
  dockWidth: number,
  mainWidth: number,
  dockMin: number,
  dockMax: number,
): number {
  const maxAllowed = Math.max(dockMin, mainWidth + dockWidth - CHAT.minWidth);
  return Math.min(dockMax, Math.max(dockMin, Math.min(dockWidth, maxAllowed)));
}
