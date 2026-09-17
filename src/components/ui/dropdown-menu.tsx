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
  /** Keep the menu open after selecting — for items that tune a setting in place. */
  closeOnSelect?: boolean;
}>) {
  return (
    <KDropdown.Item
      disabled={props.disabled}
      onSelect={props.onSelect}
      closeOnSelect={props.closeOnSelect}
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

/**
 * A row that opens a nested menu to the side. Used to keep a multi-dimension picker
 * (agent / model / effort) to one short top-level list instead of one long flat one.
 */
export function DropdownSub(props: ParentProps<{ overlap?: boolean }>) {
  return <KDropdown.Sub overlap={props.overlap}>{props.children}</KDropdown.Sub>;
}

export function DropdownSubTrigger(props: ParentProps<{ class?: string; disabled?: boolean }>) {
  return (
    <KDropdown.SubTrigger
      disabled={props.disabled}
      class={`jui-dropdown-item jui-dropdown-subtrigger ${props.class ?? ""}`}
    >
      {props.children}
    </KDropdown.SubTrigger>
  );
}

export function DropdownSubContent(props: ParentProps<{ class?: string }>) {
  return (
    <KDropdown.Portal>
      <KDropdown.SubContent class={`jui-dropdown-content jui-dropdown-subcontent ${props.class ?? ""}`}>
        {props.children}
      </KDropdown.SubContent>
    </KDropdown.Portal>
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
