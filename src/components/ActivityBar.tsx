import { For, Show } from "solid-js";
import { FileText, GitBranch } from "lucide-solid";
import { INTERACTIVE_MOTION } from "./types";

export type ActivityPanel = "git" | "files";

type ActivityBarProps = {
  activePanel: () => ActivityPanel | null;
  onSelect: (panel: ActivityPanel | null) => void;
  gitChangeCount?: () => number;
};

export default function ActivityBar(props: ActivityBarProps) {
  const items: Array<{
    id: ActivityPanel;
    title: string;
    hint: string;
    icon: () => any;
    badge?: () => number;
  }> = [
    {
      id: "files",
      title: "Explorer",
      hint: "Cmd/Ctrl+2",
      icon: () => <FileText size={18} stroke-width={1.6} />,
    },
    {
      id: "git",
      title: "Source Control",
      hint: "Cmd/Ctrl+G",
      icon: () => <GitBranch size={18} stroke-width={1.6} />,
      badge: props.gitChangeCount,
    },
  ];

  return (
    <div
      data-tauri-drag-region
      class="activity-bar w-11 shrink-0 flex flex-col items-center gap-1"
      style={{ "padding-top": "44px", "padding-bottom": "8px" }}
    >
      <For each={items}>
        {(item) => {
          const isActive = () => props.activePanel() === item.id;
          const count = () => item.badge?.() ?? 0;
          return (
            <button
              type="button"
              onClick={() => props.onSelect(isActive() ? null : item.id)}
              title={`${item.title} (${item.hint})`}
              class={`activity-button relative flex h-9 w-9 items-center justify-center ${INTERACTIVE_MOTION}`}
              classList={{ "is-active": isActive() }}
            >
              {item.icon()}
              <Show when={count() > 0}>
                <span
                  class="activity-badge absolute top-0.5 right-0.5 min-w-[14px] h-[14px] px-[3px] rounded-full flex items-center justify-center text-[9px] font-bold leading-none"
                >
                  {count() > 99 ? "99+" : count()}
                </span>
              </Show>
            </button>
          );
        }}
      </For>
    </div>
  );
}
