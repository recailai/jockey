import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";

export type BadgeTone = "neutral" | "success" | "warning" | "danger" | "info";
export type BadgeVariant = "subtle" | "outline" | "solid";

export type BadgeProps = ParentProps<JSX.HTMLAttributes<HTMLSpanElement> & {
  tone?: BadgeTone;
  variant?: BadgeVariant;
  label?: string;
}>;

export function Badge(props: BadgeProps) {
  const [local, rest] = splitProps(props, ["class", "tone", "variant", "label", "children"]);
  const variantClass = () => local.variant ? `jui-badge-${local.variant}` : "jui-badge-subtle";
  return (
    <span
      {...rest}
      class={`jui-badge jui-badge-${local.tone ?? "neutral"} ${variantClass()} ${local.class ?? ""}`}
    >
      {local.children ?? local.label}
    </span>
  );
}

