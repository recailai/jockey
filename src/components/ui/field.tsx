import { Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";

export type InputProps = JSX.InputHTMLAttributes<HTMLInputElement> & {
  error?: boolean;
  monospace?: boolean;
};

export type TextareaProps = JSX.TextareaHTMLAttributes<HTMLTextAreaElement> & {
  error?: boolean;
  monospace?: boolean;
};

export function Input(props: InputProps) {
  const [local, rest] = splitProps(props, ["class", "error", "monospace"]);
  return (
    <input
      {...rest}
      class={`jui-field ${local.error ? "is-error" : ""} ${local.monospace ? "font-mono" : ""} ${local.class ?? ""}`}
    />
  );
}

export function Textarea(props: TextareaProps) {
  const [local, rest] = splitProps(props, ["class", "error", "monospace"]);
  return (
    <textarea
      {...rest}
      class={`jui-field jui-textarea ${local.error ? "is-error" : ""} ${local.monospace ? "font-mono" : ""} ${local.class ?? ""}`}
    />
  );
}

export type FormFieldProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    label?: JSX.Element | string;
    description?: JSX.Element | string;
    error?: JSX.Element | string;
    required?: boolean;
    hint?: JSX.Element | string;
    action?: { label: string; onClick: () => void };
  }
>;

export function FormField(props: FormFieldProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "label",
    "description",
    "error",
    "required",
    "hint",
    "action",
    "children",
  ]);

  return (
    <div {...rest} class={`jui-form-field ${local.class ?? ""}`}>
      <Show when={local.label || local.action || local.hint}>
        <div class="jui-form-field-header">
          <div class="flex items-center gap-1">
            <Show when={local.label}>
              <label class="jui-form-field-label">
                {local.label}
                <Show when={local.required}>
                  <span class="text-[var(--ui-state-danger-text)] ml-0.5">*</span>
                </Show>
              </label>
            </Show>
            <Show when={local.hint}>
              <span class="jui-form-field-hint">({local.hint})</span>
            </Show>
          </div>
          <Show when={local.action}>
            {(act) => (
              <button
                type="button"
                onClick={act().onClick}
                class="jui-form-field-action"
              >
                {act().label}
              </button>
            )}
          </Show>
        </div>
      </Show>
      <div class="jui-form-field-control">{local.children}</div>
      <Show when={local.description && !local.error}>
        <p class="jui-form-field-description">{local.description}</p>
      </Show>
      <Show when={local.error}>
        <p class="jui-form-field-error">{local.error}</p>
      </Show>
    </div>
  );
}
