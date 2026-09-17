import { Show, splitProps } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import { Plus, Search, Trash2 } from "lucide-solid";
import { Button, IconButton } from "./button";

export function MasterDetailView(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <div {...rest} class={`jui-master-detail ${local.class ?? ""}`}>
      {local.children}
    </div>
  );
}

export type MasterPaneProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    title?: string;
    action?: { label: string; onClick: () => void; icon?: JSX.Element };
    search?: {
      value: string;
      onInput: (value: string) => void;
      placeholder?: string;
    };
  }
>;

export function MasterPane(props: MasterPaneProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "title",
    "action",
    "search",
    "children",
  ]);

  return (
    <div {...rest} class={`jui-master-pane ${local.class ?? ""}`}>
      <Show when={local.title || local.action}>
        <div class="jui-master-pane-header">
          <Show when={local.title}>
            <span class="jui-master-pane-title">{local.title}</span>
          </Show>
          <Show when={local.action}>
            {(act) => (
              <Button
                variant="outline"
                size="sm"
                onClick={act().onClick}
                class="jui-master-pane-action"
              >
                {act().icon ?? <Plus size={12} class="mr-1" />}
                {act().label}
              </Button>
            )}
          </Show>
        </div>
      </Show>

      <Show when={local.search}>
        {(s) => (
          <div class="jui-master-pane-search">
            <Search size={12} class="jui-master-pane-search-icon" />
            <input
              type="text"
              value={s().value}
              onInput={(e) => s().onInput(e.currentTarget.value)}
              placeholder={s().placeholder ?? "Filter…"}
              class="jui-master-pane-search-input"
            />
          </div>
        )}
      </Show>

      <div class="jui-master-pane-list">
        {local.children}
      </div>
    </div>
  );
}

export type MasterItemProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    title: string;
    subtitle?: string;
    active?: boolean;
    badge?: JSX.Element;
    onClick?: () => void;
    onDelete?: () => void;
  }
>;

export function MasterItem(props: MasterItemProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "title",
    "subtitle",
    "active",
    "badge",
    "onClick",
    "onDelete",
    "children",
  ]);

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={local.onClick}
      {...rest}
      class={`jui-master-item group ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-1.5">
          <span class="jui-master-item-title truncate">{local.title}</span>
          <Show when={local.badge}>{local.badge}</Show>
        </div>
        <Show when={local.subtitle}>
          <p class="jui-master-item-subtitle truncate">{local.subtitle}</p>
        </Show>
        {local.children}
      </div>
      <Show when={local.onDelete}>
        {(del) => (
          <IconButton
            size="sm"
            variant="ghost"
            title="Delete"
            class="jui-master-item-delete"
            onClick={(e) => {
              e.stopPropagation();
              del()();
            }}
          >
            <Trash2 size={12} />
          </IconButton>
        )}
      </Show>
    </div>
  );
}

export type DetailPaneProps = ParentProps<
  JSX.HTMLAttributes<HTMLDivElement> & {
    title?: string;
    subtitle?: string;
    actions?: JSX.Element;
    error?: string | null;
    empty?: boolean;
    emptyFallback?: JSX.Element;
  }
>;

export function DetailPane(props: DetailPaneProps) {
  const [local, rest] = splitProps(props, [
    "class",
    "title",
    "subtitle",
    "actions",
    "error",
    "empty",
    "emptyFallback",
    "children",
  ]);

  return (
    <div {...rest} class={`jui-detail-pane ${local.class ?? ""}`}>
      <Show
        when={!local.empty}
        fallback={
          local.emptyFallback ?? (
            <div class="flex flex-1 items-center justify-center text-xs theme-muted">
              Select an item to view details
            </div>
          )
        }
      >
        <Show when={local.title || local.actions}>
          <div class="jui-detail-pane-header">
            <div class="min-w-0 flex-1">
              <Show when={local.title}>
                <h3 class="jui-detail-pane-title truncate">{local.title}</h3>
              </Show>
              <Show when={local.subtitle}>
                <p class="jui-detail-pane-subtitle truncate">{local.subtitle}</p>
              </Show>
            </div>
            <Show when={local.actions}>
              <div class="flex items-center gap-2">{local.actions}</div>
            </Show>
          </div>
        </Show>

        <Show when={local.error}>
          <div class="px-5 pt-3">
            <div class="rounded-lg border border-[var(--ui-state-danger-text)]/20 bg-[var(--ui-state-danger-bg)] p-2.5 text-xs text-[var(--ui-state-danger-text)]">
              {local.error}
            </div>
          </div>
        </Show>

        <div class="jui-detail-pane-body">
          {local.children}
        </div>
      </Show>
    </div>
  );
}
