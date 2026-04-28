import { Show } from "solid-js";
import { PanelLeftClose, PanelLeftOpen } from "lucide-solid";
import { IconButton } from "../ui";

type WindowChromeProps = {
  leftRailOpen: boolean;
  onToggleLeftRail: () => void;
};

export default function WindowChrome(props: WindowChromeProps) {
  return (
    <div class="window-chrome-layer" data-tauri-drag-region>
      <IconButton
        class="window-chrome-toggle"
        onClick={props.onToggleLeftRail}
        title={props.leftRailOpen ? "Hide sidebar" : "Show sidebar"}
      >
        <Show when={props.leftRailOpen} fallback={<PanelLeftOpen size={17} />}>
          <PanelLeftClose size={17} />
        </Show>
      </IconButton>
    </div>
  );
}
