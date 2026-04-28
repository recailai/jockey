import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";

type BadgeTone = "neutral" | "success" | "warning" | "danger" | "info";

export type BadgeProps = ParentProps<JSX.HTMLAttributes<HTMLSpanElement> & {
  tone?: BadgeTone;
}>;

export function Badge(props: BadgeProps) {
  const [local, rest] = splitProps(props, ["class", "tone", "children"]);
  return (
    <span {...rest} class={`jui-badge jui-badge-${local.tone ?? "neutral"} ${local.class ?? ""}`}>
      {local.children}
    </span>
  );
}
