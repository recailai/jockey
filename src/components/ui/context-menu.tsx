import type { JSX, ParentProps } from "solid-js";
import { Show, splitProps } from "solid-js";

export function ContextMenuSurface(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement> & {
  x: number;
  y: number;
  width?: number;
}>) {
  const [local, rest] = splitProps(props, ["class", "children", "x", "y", "width"]);
  return (
    <div
      {...rest}
      class={`jui-context-menu jui-dropdown-content ${local.class ?? ""}`}
      style={{
        left: `${local.x}px`,
        top: `${local.y}px`,
        "min-width": local.width ? `${local.width}px` : undefined,
      }}
    >
      {local.children}
    </div>
  );
}

export function ContextMenuItem(props: ParentProps<{
  class?: string;
  disabled?: boolean;
  onSelect?: () => void;
  icon?: JSX.Element;
}>) {
  return (
    <button
      type="button"
      disabled={props.disabled}
      class={`jui-dropdown-item ${props.class ?? ""}`}
      onClick={props.onSelect}
    >
      <Show when={props.icon}>
        <span class="jui-dropdown-item-icon">{props.icon}</span>
      </Show>
      <span class="min-w-0 flex-1 truncate">{props.children}</span>
    </button>
  );
}

export function ContextMenuSeparator() {
  return <div class="jui-dropdown-separator" />;
}
