import { Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import * as KCheckbox from "@kobalte/core/checkbox";
import { Check, Minus } from "lucide-solid";

export type CheckboxProps = ParentProps<{
  checked?: boolean;
  defaultChecked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  indeterminate?: boolean;
  label?: JSX.Element | string;
  description?: JSX.Element | string;
  class?: string;
  id?: string;
  name?: string;
  value?: string;
}>;

export function Checkbox(props: CheckboxProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "label",
    "description",
    "checked",
    "onChange",
    "disabled",
    "indeterminate",
    "children",
  ]);

  return (
    <KCheckbox.Root
      {...rest}
      checked={local.checked}
      onChange={local.onChange}
      disabled={local.disabled}
      indeterminate={local.indeterminate}
      class={`jui-checkbox ${local.disabled ? "is-disabled" : ""} ${local.class ?? ""}`}
    >
      <KCheckbox.Input class="jui-checkbox-input" />
      <KCheckbox.Control class="jui-checkbox-control">
        <KCheckbox.Indicator class="jui-checkbox-indicator">
          <Show when={!local.indeterminate} fallback={<Minus size={11} stroke-width={2.5} />}>
            <Check size={11} stroke-width={2.5} />
          </Show>
        </KCheckbox.Indicator>
      </KCheckbox.Control>
      <Show when={local.label || local.children || local.description}>
        <div class="jui-checkbox-text">
          <Show when={local.label || local.children}>
            <KCheckbox.Label class="jui-checkbox-label">
              {local.label ?? local.children}
            </KCheckbox.Label>
          </Show>
          <Show when={local.description}>
            <KCheckbox.Description class="jui-checkbox-description">
              {local.description}
            </KCheckbox.Description>
          </Show>
        </div>
      </Show>
    </KCheckbox.Root>
  );
}
