import type { AppSession, PreviewTab, PreviewMode } from "../components/types";

type MutateSession = (id: string, recipe: (s: AppSession) => void) => void;

export const tabIdFor = (sessionId: string, cwd: string, path: string): string =>
  `${sessionId}|${cwd}|${path}`;

const labelFor = (path: string): string => {
  const trimmed = path.endsWith("/") ? path.slice(0, -1) : path;
  const slash = trimmed.lastIndexOf("/");
  const name = slash === -1 ? trimmed : trimmed.slice(slash + 1);
  return path.endsWith("/") ? `${name}/` : name;
};

export type OpenPreviewTabInput = {
  cwd: string;
  path: string;
  initialMode: PreviewMode;
  staged: boolean;
  untracked: boolean;
  commitOid?: string | null;
  label?: string;
};

export const openPreviewTab = (
  mutateSession: MutateSession,
  sessionId: string,
  opts: OpenPreviewTabInput,
): void => {
  const id = tabIdFor(sessionId, opts.cwd, opts.path);
  mutateSession(sessionId, (s) => {
    const existing = s.previewTabs.find((t) => t.id === id);
    if (!existing) {
      const tab: PreviewTab = {
        id,
        cwd: opts.cwd,
        path: opts.path,
        label: opts.label ?? labelFor(opts.path),
        initialMode: opts.initialMode,
        staged: opts.staged,
        untracked: opts.untracked,
        commitOid: opts.commitOid ?? null,
      };
      s.previewTabs.push(tab);
    }
    s.activePreviewTabId = id;
  });
};

export const closePreviewTab = (
  mutateSession: MutateSession,
  sessionId: string,
  tabId: string,
): void => {
  mutateSession(sessionId, (s) => {
    const idx = s.previewTabs.findIndex((t) => t.id === tabId);
    if (idx === -1) return;
    s.previewTabs.splice(idx, 1);
    if (s.activePreviewTabId === tabId) {
      const fallback = s.previewTabs[idx] ?? s.previewTabs[idx - 1] ?? null;
      s.activePreviewTabId = fallback?.id ?? null;
    }
  });
};

export const setActivePreviewTab = (
  mutateSession: MutateSession,
  sessionId: string,
  tabId: string,
): void => {
  mutateSession(sessionId, (s) => {
    if (s.previewTabs.some((t) => t.id === tabId)) {
      s.activePreviewTabId = tabId;
    }
  });
};

export const closeOtherPreviewTabs = (
  mutateSession: MutateSession,
  sessionId: string,
  keepTabId: string,
): void => {
  mutateSession(sessionId, (s) => {
    s.previewTabs = s.previewTabs.filter((t) => t.id === keepTabId);
    s.activePreviewTabId = s.previewTabs[0]?.id ?? null;
  });
};

export const closeAllPreviewTabs = (
  mutateSession: MutateSession,
  sessionId: string,
): void => {
  mutateSession(sessionId, (s) => {
    s.previewTabs = [];
    s.activePreviewTabId = null;
  });
};
