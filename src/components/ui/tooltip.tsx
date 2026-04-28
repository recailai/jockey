import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";
import * as KTooltip from "@kobalte/core/tooltip";

export function Tooltip(props: ParentProps<{ label: string; placement?: "top" | "bottom" | "left" | "right" }>) {
  return (
    <KTooltip.Root placement={props.placement ?? "bottom"} gutter={6} openDelay={450} closeDelay={80}>
      <KTooltip.Trigger>{props.children}</KTooltip.Trigger>
      <KTooltip.Portal>
        <KTooltip.Content class="jui-tooltip">
          {props.label}
        </KTooltip.Content>
      </KTooltip.Portal>
    </KTooltip.Root>
  );
}

export function TooltipContent(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-tooltip ${local.class ?? ""}`}>{local.children}</div>;
}
