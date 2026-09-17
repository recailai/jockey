import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";

export type PillProps = ParentProps<
  JSX.HTMLAttributes<HTMLElement> & {
    active?: boolean;
    onClick?: (e: MouseEvent) => void;
    disabled?: boolean;
  }
>;

export function Pill(props: PillProps) {
  const [local, rest] = splitProps(props, ["class", "active", "onClick", "disabled", "children"]);

  if (local.onClick) {
    return (
      <button
        type="button"
        disabled={local.disabled}
        onClick={local.onClick}
        {...(rest as JSX.ButtonHTMLAttributes<HTMLButtonElement>)}
        class={`jui-pill jui-pill-interactive ${local.active ? "is-active" : ""} ${local.disabled ? "is-disabled" : ""} ${local.class ?? ""}`}
      >
        {local.children}
      </button>
    );
  }

  return (
    <span
      {...rest}
      class={`jui-pill ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </span>
  );
}
