import { Show } from "solid-js";
import type { JSX } from "solid-js";
import { Folder, GitBranch, Globe, Terminal as TerminalIcon } from "lucide-solid";
import { openUrl } from "@tauri-apps/plugin-opener";
import { DockTabStrip } from "../ui";
import type { RightDockPanel } from "../../lib/layoutTokens";

type RightToolDockProps = {
  open: boolean;
  activePanel: RightDockPanel | null;
  widthPx: number;
  previewPx: number | null;
  onResizeStart: (event: MouseEvent) => void;
  onPanelChange: (panel: RightDockPanel) => void;
  onShowLauncher: () => void;
  children: JSX.Element;
};

const TABS = [
  { id: "files" as const, label: "Files", icon: Folder, shortcut: "⌘1" },
  { id: "git" as const, label: "Git", icon: GitBranch, shortcut: "⌘2" },
  { id: "terminal" as const, label: "Terminal", icon: TerminalIcon, shortcut: "⌘3" },
];

const LAUNCHER_CARDS: Array<{
  id: RightDockPanel | "browser";
  title: string;
  hint: string;
  shortcut?: string;
  icon: typeof Folder;
}> = [
  { id: "files", title: "Files", hint: "Browse project files", shortcut: "⌘1", icon: Folder },
  { id: "git", title: "Git", hint: "Changes, commit, and history", shortcut: "⌘2", icon: GitBranch },
  { id: "browser", title: "Browser", hint: "Open a website", icon: Globe },
  { id: "terminal", title: "Terminal", hint: "Run shell commands", shortcut: "⌘3", icon: TerminalIcon },
];

export default function RightToolDock(props: RightToolDockProps) {
  const openBrowser = async () => {
    const url = window.prompt("Open URL in browser", "https://");
    if (!url?.trim()) return;
    try {
      await openUrl(url.trim());
    } catch {
      // ignore
    }
  };

  const handleCardClick = (id: RightDockPanel | "browser") => {
    if (id === "browser") {
      void openBrowser();
      return;
    }
    props.onPanelChange(id);
  };

  return (
    <Show when={props.open}>
      <div
        class="resizer-x right-dock-resizer"
        onMouseDown={props.onResizeStart}
        title="Drag to resize panel"
      />
      <aside class="right-tool-dock" style={{ width: `${props.widthPx}px` }}>
        <Show when={props.activePanel !== null} fallback={
          <div class="right-dock-cards">
            {LAUNCHER_CARDS.map((card) => {
              const Icon = card.icon;
              return (
                <button type="button" class="right-dock-card" onClick={() => handleCardClick(card.id)}>
                  <span class="right-dock-card-icon">
                    <Icon size={17} stroke-width={1.75} />
                  </span>
                  <span class="right-dock-card-copy">
                    <span class="right-dock-card-title">{card.title}</span>
                    <span class="right-dock-card-hint">{card.hint}</span>
                  </span>
                  <Show when={card.shortcut}>
                    <kbd class="right-dock-card-kbd">{card.shortcut}</kbd>
                  </Show>
                </button>
              );
            })}
          </div>
        }>
          <DockTabStrip
            tabs={TABS}
            activeId={props.activePanel!}
            onTabChange={(id) => props.onPanelChange(id as RightDockPanel)}
            onBack={props.onShowLauncher}
            backTitle="Back to quick tools"
          />
          <div class="right-dock-body">
            {props.children}
          </div>
        </Show>
      </aside>
      <Show when={props.previewPx !== null}>
        <div
          class="resize-guide-x resize-guide-x-from-right"
          style={{ right: `${props.previewPx ?? 0}px` }}
        />
      </Show>
    </Show>
  );
}
