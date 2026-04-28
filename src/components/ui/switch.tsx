import type { ParentProps } from "solid-js";
import * as KSwitch from "@kobalte/core/switch";

export function Switch(props: ParentProps<{
  checked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
}>) {
  return (
    <KSwitch.Root
      checked={props.checked}
      onChange={props.onChange}
      disabled={props.disabled}
      class="jui-switch"
    >
      <KSwitch.Input />
      <KSwitch.Control class="jui-switch-control">
        <KSwitch.Thumb class="jui-switch-thumb" />
      </KSwitch.Control>
      {props.label ? <KSwitch.Label class="jui-switch-label">{props.label}</KSwitch.Label> : props.children}
    </KSwitch.Root>
  );
}
