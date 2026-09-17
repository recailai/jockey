import { Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import { Button } from "./button";

export type EmptyStateProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    icon?: JSX.Element | string;
    title?: string;
    description?: string;
    action?: { label: string; onClick: () => void };
  }
>;

export function EmptyState(props: EmptyStateProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "icon",
    "title",
    "description",
    "action",
    "children",
  ]);

  return (
    <div {...rest} class={`jui-empty-state ${local.class ?? ""}`}>
      <Show when={local.icon}>
        <div class="jui-empty-state-icon">
          {typeof local.icon === "string" ? (
            <span class="text-3xl opacity-25 select-none">{local.icon}</span>
          ) : (
            local.icon
          )}
        </div>
      </Show>
      <Show when={local.title}>
        <h4 class="jui-empty-state-title">{local.title}</h4>
      </Show>
      <Show when={local.description}>
        <p class="jui-empty-state-description">{local.description}</p>
      </Show>
      <Show when={local.children}>
        {local.children}
      </Show>
      <Show when={local.action}>
        {(act) => (
          <div class="mt-3">
            <Button variant="outline" size="sm" onClick={act().onClick}>
              {act().label}
            </Button>
          </div>
        )}
      </Show>
    </div>
  );
}
