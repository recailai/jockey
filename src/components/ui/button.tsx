import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";

type ButtonVariant = "default" | "secondary" | "outline" | "ghost" | "destructive" | "primary" | "danger";
type ButtonSize = "sm" | "md" | "lg";

export type ButtonProps = ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
}>;

export function Button(props: ButtonProps) {
  const [local, rest] = splitProps(props, ["class", "variant", "size", "children"]);
  return (
    <button
      type="button"
      {...rest}
      class={`jui-button jui-button-${local.variant ?? "default"} jui-button-${local.size ?? "md"} ${local.class ?? ""}`}
    >
      {local.children}
    </button>
  );
}

export type IconButtonProps = ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  size?: "sm" | "md";
  variant?: "default" | "ghost" | "outline";
}>;

export function IconButton(props: IconButtonProps) {
  const [local, rest] = splitProps(props, ["class", "active", "size", "variant", "children"]);
  return (
    <button
      type="button"
      {...rest}
      class={`jui-icon-button jui-icon-button-${local.size ?? "md"} jui-icon-button-${local.variant ?? "ghost"} ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </button>
  );
}

export type ToolbarButtonProps = ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  variant?: "default" | "ghost" | "outline";
}>;

export function ToolbarButton(props: ToolbarButtonProps) {
  const [local, rest] = splitProps(props, ["class", "active", "variant", "children"]);
  return (
    <button
      type="button"
      {...rest}
      class={`jui-toolbar-button jui-toolbar-button-${local.variant ?? "ghost"} ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </button>
  );
}

export type RowButtonProps = ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
}>;

export function RowButton(props: RowButtonProps) {
  const [local, rest] = splitProps(props, ["class", "active", "children"]);
  return (
    <button
      type="button"
      {...rest}
      class={`jui-row-button ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </button>
  );
}

export type ListRowProps = ParentProps<JSX.HTMLAttributes<HTMLDivElement> & {
  active?: boolean;
  disabled?: boolean;
}>;

export function ListRow(props: ListRowProps) {
  const [local, rest] = splitProps(props, ["class", "active", "disabled", "children"]);
  return (
    <div
      {...rest}
      class={`jui-list-row ${local.active ? "is-active" : ""} ${local.disabled ? "is-disabled" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </div>
  );
}

export function SplitButton(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-split-button ${local.class ?? ""}`}>{local.children}</div>;
}

export function SplitButtonMain(props: ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <button type="button" {...rest} class={`jui-split-button-main ${local.class ?? ""}`}>
      {local.children}
    </button>
  );
}

export function SplitButtonTrigger(props: ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <button type="button" {...rest} class={`jui-split-button-trigger ${local.class ?? ""}`}>
      {local.children}
    </button>
  );
}
