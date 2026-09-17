import { Match, Show, Switch } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import { Alert } from "./alert";
import { EmptyState } from "./empty-state";

export type AsyncContentState = "loading" | "error" | "empty" | "ready";

export type AsyncContentProps = ParentProps<{
  state: AsyncContentState;
  loadingFallback?: JSX.Element;
  emptyFallback?: JSX.Element;
  emptyTitle?: string;
  emptyDescription?: string;
  emptyIcon?: JSX.Element | string;
  error?: string | null;
  onRetry?: () => void;
  class?: string;
}>;

export function AsyncContent(props: AsyncContentProps) {
  return (
    <Switch>
      <Match when={props.state === "loading"}>
        <Show
          when={props.loadingFallback}
          fallback={
            <div class={`flex items-center justify-center py-12 text-xs theme-muted ${props.class ?? ""}`}>
              <span class="inline-block animate-spin mr-2">⟳</span> Loading…
            </div>
          }
        >
          {props.loadingFallback}
        </Show>
      </Match>

      <Match when={props.state === "error"}>
        <div class={`p-4 ${props.class ?? ""}`}>
          <Alert
            tone="danger"
            title="Failed to load"
            action={props.onRetry ? { label: "Retry", onClick: props.onRetry } : undefined}
          >
            {props.error ?? "An unexpected error occurred while loading content."}
          </Alert>
        </div>
      </Match>

      <Match when={props.state === "empty"}>
        <Show
          when={props.emptyFallback}
          fallback={
            <EmptyState
              icon={props.emptyIcon}
              title={props.emptyTitle ?? "No items"}
              description={props.emptyDescription}
              class={props.class}
            />
          }
        >
          {props.emptyFallback}
        </Show>
      </Match>

      <Match when={props.state === "ready"}>
        {props.children}
      </Match>
    </Switch>
  );
}
