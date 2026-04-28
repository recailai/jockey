import type { JSX, ParentProps } from "solid-js";
import { Show, splitProps } from "solid-js";
import * as KDialog from "@kobalte/core/dialog";
import { X } from "lucide-solid";
import { IconButton } from "./button";

export function Dialog(props: ParentProps<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
}>) {
  return (
    <KDialog.Root open={props.open} onOpenChange={props.onOpenChange}>
      {props.children}
    </KDialog.Root>
  );
}

export function DialogContent(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement> & {
  title?: string;
  description?: string;
  icon?: JSX.Element;
}>) {
  const [local, rest] = splitProps(props, ["class", "children", "title", "description", "icon"]);
  return (
    <KDialog.Portal>
      <KDialog.Overlay class="jui-dialog-overlay" />
      <KDialog.Content {...rest} class={`jui-dialog-content ${local.class ?? ""}`}>
        <div class="jui-dialog-top">
          <Show when={local.icon}>
            <div class="jui-dialog-icon">{local.icon}</div>
          </Show>
          <KDialog.CloseButton as={IconButton} class="ml-auto" title="Close">
            <X size={17} />
          </KDialog.CloseButton>
        </div>
        <Show when={local.title || local.description}>
          <div class="jui-dialog-heading">
            <Show when={local.title}>
              <KDialog.Title class="jui-dialog-title">{local.title}</KDialog.Title>
            </Show>
            <Show when={local.description}>
              <KDialog.Description class="jui-dialog-description">{local.description}</KDialog.Description>
            </Show>
          </div>
        </Show>
        {local.children}
      </KDialog.Content>
    </KDialog.Portal>
  );
}
