import type { JSX, ParentProps } from "solid-js";
import { Show, splitProps } from "solid-js";
import * as KDropdown from "@kobalte/core/dropdown-menu";

export function DropdownMenu(props: ParentProps<{
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  modal?: boolean;
}>) {
  return (
    <KDropdown.Root
      open={props.open}
      defaultOpen={props.defaultOpen}
      onOpenChange={props.onOpenChange}
      modal={props.modal}
    >
      {props.children}
    </KDropdown.Root>
  );
}

export function DropdownTrigger(props: ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  variant?: "toolbar" | "button" | "plain";
}>) {
  const [local, rest] = splitProps(props, ["class", "children", "active", "variant"]);
  const base = local.variant === "toolbar"
    ? "jui-toolbar-button"
    : local.variant === "button"
      ? "jui-button jui-button-default jui-button-md"
      : "";
  return (
    <KDropdown.Trigger
      {...rest}
      class={`${base} ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </KDropdown.Trigger>
  );
}

export function DropdownContent(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement> & {
  placement?: "bottom-start" | "bottom-end" | "top-start" | "top-end" | "right-start" | "left-start";
  gutter?: number;
  sameWidth?: boolean;
}>) {
  const [local, rest] = splitProps(props, ["class", "children", "placement", "gutter", "sameWidth"]);
  return (
    <KDropdown.Portal>
      <KDropdown.Content
        {...(rest as any)}
        placement={local.placement ?? "bottom-end"}
        gutter={local.gutter ?? 6}
        sameWidth={local.sameWidth}
        class={`jui-dropdown-content ${local.class ?? ""}`}
      >
        {local.children}
      </KDropdown.Content>
    </KDropdown.Portal>
  );
}

export function DropdownItem(props: ParentProps<{
  class?: string;
  disabled?: boolean;
  destructive?: boolean;
  onSelect?: () => void;
  icon?: JSX.Element;
  title?: string;
}>) {
  return (
    <KDropdown.Item
      disabled={props.disabled}
      onSelect={props.onSelect}
      title={props.title}
      class={`jui-dropdown-item ${props.destructive ? "is-destructive" : ""} ${props.class ?? ""}`}
    >
      <Show when={props.icon}>
        <span class="jui-dropdown-item-icon">{props.icon}</span>
      </Show>
      <Show when={props.icon} fallback={props.children}>
        <span class="min-w-0 flex-1 truncate">{props.children}</span>
      </Show>
    </KDropdown.Item>
  );
}

export function DropdownLabel(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-dropdown-label ${local.class ?? ""}`}>{local.children}</div>;
}

export function DropdownDescription(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-dropdown-description ${local.class ?? ""}`}>{local.children}</div>;
}

export function DropdownSeparator() {
  return <KDropdown.Separator class="jui-dropdown-separator" />;
}
