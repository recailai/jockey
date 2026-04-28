import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";
import { IconButton, type IconButtonProps } from "./button";

export function Panel(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-panel ${local.class ?? ""}`}>{local.children}</div>;
}

export function PanelHeader(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-panel-header ${local.class ?? ""}`}>{local.children}</div>;
}

export function PanelBody(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-panel-body ${local.class ?? ""}`}>{local.children}</div>;
}

export function PanelHeaderAction(props: IconButtonProps) {
  return <IconButton size="sm" {...props} class={`jui-panel-header-action ${props.class ?? ""}`} />;
}

export function EmptyState(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-empty-state ${local.class ?? ""}`}>{local.children}</div>;
}
