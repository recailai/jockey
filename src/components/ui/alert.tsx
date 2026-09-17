import { Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  Info,
  X,
} from "lucide-solid";

export type AlertTone = "neutral" | "info" | "success" | "warning" | "danger";

export type AlertProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    tone?: AlertTone;
    title?: string;
    icon?: JSX.Element;
    action?: { label: string; onClick: () => void };
    onClose?: () => void;
  }
>;

export function Alert(props: AlertProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "tone",
    "title",
    "icon",
    "action",
    "onClose",
    "children",
  ]);

  const tone = () => local.tone ?? "info";

  const defaultIcon = () => {
    switch (tone()) {
      case "success":
        return <CheckCircle2 size={16} class="shrink-0" />;
      case "warning":
        return <AlertTriangle size={16} class="shrink-0" />;
      case "danger":
        return <AlertCircle size={16} class="shrink-0" />;
      case "info":
      case "neutral":
      default:
        return <Info size={16} class="shrink-0" />;
    }
  };

  return (
    <div
      role="alert"
      {...rest}
      class={`jui-alert jui-alert-${tone()} ${local.class ?? ""}`}
    >
      <div class="jui-alert-icon">{local.icon ?? defaultIcon()}</div>
      <div class="jui-alert-content">
        <Show when={local.title}>
          <div class="jui-alert-title">{local.title}</div>
        </Show>
        <Show when={local.children}>
          <div class="jui-alert-body">{local.children}</div>
        </Show>
      </div>
      <Show when={local.action}>
        {(act) => (
          <button
            type="button"
            onClick={act().onClick}
            class="jui-alert-action"
          >
            {act().label}
          </button>
        )}
      </Show>
      <Show when={local.onClose}>
        {(close) => (
          <button
            type="button"
            onClick={close()}
            class="jui-alert-close"
            title="Dismiss"
          >
            <X size={14} />
          </button>
        )}
      </Show>
    </div>
  );
}
