import { Folder, GitBranch, Terminal as TerminalIcon } from "lucide-solid";
import type { LeftDockPanel } from "./layoutTokens";

export const LEFT_DOCK_TABS: Array<{
  id: LeftDockPanel;
  label: string;
  icon: typeof Folder;
  shortcut: string;
}> = [
  { id: "files", label: "Files", icon: Folder, shortcut: "⌘1" },
  { id: "git", label: "Git", icon: GitBranch, shortcut: "⌘2" },
  { id: "terminal", label: "Terminal", icon: TerminalIcon, shortcut: "⌘3" },
];

export function isLeftDockTabActive(active: LeftDockPanel, tabId: LeftDockPanel): boolean {
  return active === tabId || (tabId === "git" && active === "commit");
}
