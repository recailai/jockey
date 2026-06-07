import { Show, Suspense, lazy } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession } from "../types";
import type { GitStatusStore } from "../../hooks/useGitPoller";
import type { LeftDockPanel } from "../../lib/layoutTokens";
import TerminalPanel from "../TerminalPanel";

const GitPanel = lazy(() => import("../GitPanel"));
const WorkspaceFilesPanel = lazy(() => import("../WorkspaceFilesPanel"));

type ToolDockPanelsProps = {
  activePanel: LeftDockPanel | null;
  dockEmbedded?: boolean;
  activeSession: Accessor<AppSession | null>;
  gitStatus: () => GitStatusStore;
  onRefreshGit: () => void;
  onClose: () => void;
  onAddMention: (path: string) => void;
  onOpenFile: (path: string) => void;
  onOpenDiff: (path: string, staged: boolean, untracked: boolean) => void;
  onOpenCommitDiff: (oid: string, label: string) => void;
  terminalCommandRequest: { id: number; command: string } | null;
};

export default function ToolDockPanels(props: ToolDockPanelsProps) {
  return (
    <div class="tool-dock-panels relative flex min-h-0 flex-1 flex-col">
    <Suspense fallback={<div class="dock-panel-skeleton flex-1" />}>
      <Show when={props.activePanel === "git" || props.activePanel === "commit"}>
        <div class="tool-dock-panel-layer flex min-h-0 flex-1 flex-col">
        <GitPanel
          dockEmbedded={props.dockEmbedded}
          appSessionId={() => props.activeSession()?.id}
          cwd={() => props.activeSession()?.cwd ?? null}
          gitStatus={props.gitStatus}
          onRefresh={props.onRefreshGit}
          onAddMention={props.onAddMention}
          onCollapse={props.onClose}
          onOpenDiff={props.onOpenDiff}
          onOpenCommitDiff={props.onOpenCommitDiff}
        />
        </div>
      </Show>
      <Show when={props.activePanel === "files"}>
        <div class="tool-dock-panel-layer flex min-h-0 flex-1 flex-col">
        <WorkspaceFilesPanel
          dockEmbedded={props.dockEmbedded}
          appSessionId={() => props.activeSession()?.id}
          cwd={() => props.activeSession()?.cwd ?? null}
          gitStatus={props.gitStatus}
          onRefreshGit={props.onRefreshGit}
          onOpenFile={props.onOpenFile}
          onOpenDiff={props.onOpenDiff}
          onCollapse={props.onClose}
        />
        </div>
      </Show>
      <TerminalPanel
        dockEmbedded={props.dockEmbedded}
        visible={props.activePanel === "terminal"}
        activeSession={props.activeSession}
        commandRequest={props.terminalCommandRequest}
        onClose={props.onClose}
      />
    </Suspense>
    </div>
  );
}
