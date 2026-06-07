const STORAGE_KEY = "jockey:ui.prefs";

export type WorkMode = "coding" | "everyday";
export type LastRightPanel = "browser" | "terminal";

export type CustomWorkspaceApp = {
  id: string;
  label: string;
  /** macOS .app display name for `open -a` */
  appName: string;
  bundleId?: string;
};

export type UiPrefs = {
  workMode: WorkMode;
  defaultPermissions: boolean;
  autoReview: boolean;
  fullAccess: boolean;
  showBranchState: boolean;
  openDiffsInMain: boolean;
  lastRightPanel: LastRightPanel;
  customWorkspaceApps: CustomWorkspaceApp[];
};

const DEFAULTS: UiPrefs = {
  workMode: "coding",
  defaultPermissions: true,
  autoReview: true,
  fullAccess: false,
  showBranchState: true,
  openDiffsInMain: true,
  lastRightPanel: "terminal",
  customWorkspaceApps: [],
};

function normalizeCustomApps(raw: unknown): CustomWorkspaceApp[] {
  if (!Array.isArray(raw)) return [];
  const out: CustomWorkspaceApp[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const id = typeof o.id === "string" ? o.id.trim() : "";
    const label = typeof o.label === "string" ? o.label.trim() : "";
    const appName = typeof o.appName === "string" ? o.appName.trim() : "";
    if (!id || !label || !appName) continue;
    const bundleId = typeof o.bundleId === "string" ? o.bundleId.trim() : "";
    out.push({
      id,
      label,
      appName,
      ...(bundleId ? { bundleId } : {}),
    });
  }
  return out;
}

function normalize(raw: unknown): UiPrefs {
  if (!raw || typeof raw !== "object") return { ...DEFAULTS };
  const o = raw as Record<string, unknown>;
  return {
    workMode: o.workMode === "everyday" ? "everyday" : "coding",
    defaultPermissions: o.defaultPermissions !== false,
    autoReview: o.autoReview !== false,
    fullAccess: o.fullAccess === true,
    showBranchState: o.showBranchState !== false,
    openDiffsInMain: o.openDiffsInMain !== false,
    lastRightPanel: o.lastRightPanel === "browser" ? "browser" : "terminal",
    customWorkspaceApps: normalizeCustomApps(o.customWorkspaceApps),
  };
}

export function loadUiPrefs(): UiPrefs {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) return normalize(JSON.parse(raw));
  } catch { /* ignore */ }
  return { ...DEFAULTS };
}

export function saveUiPrefs(patch: Partial<UiPrefs>): UiPrefs {
  const next = { ...loadUiPrefs(), ...patch };
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch { /* ignore */ }
  return next;
}
