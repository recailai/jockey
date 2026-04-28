import type { JSX } from "solid-js";
import { Show } from "solid-js";

type ToolPanelDockProps = {
  open: boolean;
  widthPx: number;
  previewPx: number | null;
  onResizeStart: (event: MouseEvent) => void;
  children: JSX.Element;
};

export default function ToolPanelDock(props: ToolPanelDockProps) {
  return (
    <Show when={props.open}>
      <div
        class="resizer-x tool-panel-resizer"
        onMouseDown={props.onResizeStart}
        title="Drag to resize panel"
      />
      <Show when={props.previewPx !== null}>
        <div
          class="pointer-events-none fixed bottom-0 top-0 w-px bg-[var(--ui-accent)] opacity-70 z-[70]"
          style={{ right: `${props.previewPx ?? 0}px` }}
        />
      </Show>
      <aside class="right-tool-panel" style={{ width: `${props.widthPx}px` }}>
        {props.children}
      </aside>
    </Show>
  );
}
