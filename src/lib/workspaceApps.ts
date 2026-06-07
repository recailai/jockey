import type { WorkspaceOpenTarget } from "./tauriApi";
import type { CustomWorkspaceApp } from "./uiPrefs";
import { loadUiPrefs } from "./uiPrefs";

export type { CustomWorkspaceApp };

export type WorkspaceAppOption = {
  target: WorkspaceOpenTarget;
  label: string;
  /** macOS .app display name for `open -a` */
  appName: string;
  bundleId?: string;
  /** Bundled icon under `public/icons/` */
  icon?: string;
  /** Fallback monogram when icon unavailable */
  fallback: string;
  tone: string;
};

export const CUSTOM_TARGET_PREFIX = "custom:";

export const BUILTIN_WORKSPACE_APPS: WorkspaceAppOption[] = [
  { target: "vscode", label: "VS Code", appName: "Visual Studio Code", icon: "/icons/vscode.png", fallback: "VS", tone: "vscode" },
  { target: "cursor", label: "Cursor", appName: "Cursor", icon: "/icons/cursor.png", fallback: "C", tone: "cursor" },
  {
    target: "antigravity",
    label: "Antigravity",
    appName: "Antigravity IDE",
    bundleId: "com.google.antigravity-ide",
    icon: "/icons/antigravity.png",
    fallback: "A",
    tone: "antigravity",
  },
  { target: "finder", label: "Finder", appName: "Finder", icon: "/icons/finder.png", fallback: "F", tone: "finder" },
  { target: "terminal", label: "Terminal", appName: "Terminal", icon: "/icons/terminal.png", fallback: ">", tone: "terminal" },
];

export function customWorkspaceTarget(id: string): WorkspaceOpenTarget {
  return `${CUSTOM_TARGET_PREFIX}${id}`;
}

export function isCustomWorkspaceTarget(target: string): boolean {
  return target.startsWith(CUSTOM_TARGET_PREFIX);
}

export function listWorkspaceApps(customApps: CustomWorkspaceApp[] = loadUiPrefs().customWorkspaceApps): WorkspaceAppOption[] {
  const custom = customApps.map((app): WorkspaceAppOption => ({
    target: customWorkspaceTarget(app.id),
    label: app.label,
    appName: app.appName,
    bundleId: app.bundleId,
    fallback: app.label.trim().charAt(0).toUpperCase() || "?",
    tone: "custom",
  }));
  return [...BUILTIN_WORKSPACE_APPS, ...custom];
}

export function workspaceAppFor(
  target: WorkspaceOpenTarget,
  customApps: CustomWorkspaceApp[] = loadUiPrefs().customWorkspaceApps,
): WorkspaceAppOption {
  return listWorkspaceApps(customApps).find((a) => a.target === target) ?? BUILTIN_WORKSPACE_APPS[0];
}

export function isKnownWorkspaceTarget(
  target: string,
  customApps: CustomWorkspaceApp[] = loadUiPrefs().customWorkspaceApps,
): boolean {
  return listWorkspaceApps(customApps).some((a) => a.target === target);
}
