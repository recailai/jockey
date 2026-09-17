import { createSignal, Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import * as KDialog from "@kobalte/core/dialog";
import { AlertTriangle, X } from "lucide-solid";
import { Button, IconButton } from "./button";
import { Checkbox } from "./checkbox";

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

export function DialogBody(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <div {...rest} class={`jui-dialog-body ${local.class ?? ""}`}>
      {local.children}
    </div>
  );
}

export type DialogFooterProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    cancelText?: string;
    confirmText?: string;
    onCancel?: () => void;
    onConfirm?: () => void;
    confirmDisabled?: boolean;
    confirmLoading?: boolean;
    confirmVariant?: "default" | "destructive" | "primary" | "secondary";
  }
>;

export function DialogFooter(props: DialogFooterProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "cancelText",
    "confirmText",
    "onCancel",
    "onConfirm",
    "confirmDisabled",
    "confirmLoading",
    "confirmVariant",
    "children",
  ]);

  return (
    <div {...rest} class={`jui-dialog-footer ${local.class ?? ""}`}>
      <Show when={local.children} fallback={
        <>
          <Show when={local.onCancel}>
            <Button
              variant="secondary"
              size="md"
              onClick={local.onCancel}
            >
              {local.cancelText ?? "Cancel"}
            </Button>
          </Show>
          <Show when={local.onConfirm}>
            <Button
              variant={local.confirmVariant ?? "default"}
              size="md"
              disabled={local.confirmDisabled || local.confirmLoading}
              onClick={local.onConfirm}
            >
              {local.confirmLoading ? "Processing…" : (local.confirmText ?? "Confirm")}
            </Button>
          </Show>
        </>
      }>
        {local.children}
      </Show>
    </div>
  );
}

export type RiskConfirmationProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  acknowledgeLabel: string;
  cancelLabel?: string;
  confirmLabel?: string;
  onConfirm: () => void | Promise<void>;
  loading?: boolean;
};

export function RiskConfirmation(props: RiskConfirmationProps) {
  const [acknowledged, setAcknowledged] = createSignal(false);

  const handleClose = () => {
    setAcknowledged(false);
    props.onOpenChange(false);
  };

  const handleConfirm = async () => {
    if (!acknowledged()) return;
    await props.onConfirm();
    handleClose();
  };

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        title={props.title}
        icon={<AlertTriangle size={20} class="text-[var(--ui-state-danger-text)]" />}
      >
        <DialogBody class="space-y-4">
          <p class="text-xs leading-relaxed theme-muted">{props.description}</p>
          <div class="rounded-lg border border-[var(--ui-border)] p-3 bg-[var(--ui-surface-muted)]">
            <Checkbox
              checked={acknowledged()}
              onChange={setAcknowledged}
              label={props.acknowledgeLabel}
            />
          </div>
        </DialogBody>
        <DialogFooter
          cancelText={props.cancelLabel ?? "Cancel"}
          confirmText={props.confirmLabel ?? "Confirm"}
          confirmVariant="destructive"
          confirmDisabled={!acknowledged()}
          confirmLoading={props.loading}
          onCancel={handleClose}
          onConfirm={() => void handleConfirm()}
        />
      </DialogContent>
    </Dialog>
  );
}
