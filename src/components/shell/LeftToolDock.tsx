import { Show } from "solid-js";
import type { JSX } from "solid-js";
import type { LeftDockPanel } from "../../lib/layoutTokens";

type LeftToolDockProps = {
  open: boolean;
  activePanel: LeftDockPanel;
  widthPx: number;
  previewPx: number | null;
  onResizeStart: (event: MouseEvent) => void;
  children: JSX.Element;
};

export default function LeftToolDock(props: LeftToolDockProps) {
  return (
    <Show when={props.open}>
      <aside class="tool-panel-dock left-tool-dock" style={{ width: `${props.widthPx}px` }}>
        <div class="left-dock-body">
          {props.children}
        </div>
      </aside>
      <div
        class="resizer-x tool-panel-resizer"
        onMouseDown={props.onResizeStart}
        title="Drag to resize panel"
      />
      <Show when={props.previewPx !== null}>
        <div
          class="resize-guide-x"
          style={{ left: `${props.previewPx ?? 0}px` }}
        />
      </Show>
    </Show>
  );
}
