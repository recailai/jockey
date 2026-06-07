import type { JSX, ParentProps } from "solid-js";
import { For, Show, splitProps } from "solid-js";
import { ChevronLeft, type LucideIcon } from "lucide-solid";
import { ToolbarButton } from "./button";
import { Panel, PanelBody, PanelHeaderAction } from "./panel";

/** Full-height panel shell for left/right tool docks. */
export function DockPanel(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <Panel {...rest} class={`dock-panel tool-panel flex h-full flex-col overflow-hidden ${local.class ?? ""}`}>
      {local.children}
    </Panel>
  );
}

/** Compact in-panel toolbar (embedded dock mode). */
export function DockPanelToolbar(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <div {...rest} class={`dock-panel-toolbar ${local.class ?? ""}`}>
      {local.children}
    </div>
  );
}

export function DockPanelToolbarTitle(props: ParentProps<JSX.HTMLAttributes<HTMLSpanElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <span {...rest} class={`dock-panel-toolbar-title ${local.class ?? ""}`}>
      {local.children}
    </span>
  );
}

export function DockPanelToolbarActions(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <div {...rest} class={`dock-panel-toolbar-actions ${local.class ?? ""}`}>
      {local.children}
    </div>
  );
}

/** Scrollable dock panel body with consistent padding. */
export function DockPanelBody(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <PanelBody {...rest} class={`dock-panel-body flex-1 min-h-0 overflow-auto ${local.class ?? ""}`}>
      {local.children}
    </PanelBody>
  );
}

export type DockTabItem = {
  id: string;
  label: string;
  icon: LucideIcon;
  shortcut?: string;
};

type DockTabStripProps = {
  tabs: DockTabItem[];
  activeId: string;
  onTabChange: (id: string) => void;
  onBack?: () => void;
  backTitle?: string;
};

/** Icon-only tab strip for dock panel headers (right dock, etc.). */
export function DockTabStrip(props: DockTabStripProps) {
  return (
    <DockPanelToolbar class="dock-tab-strip">
      <div class="dock-tab-strip-tabs">
        <For each={props.tabs}>
          {(tab) => {
            const Icon = tab.icon;
            return (
              <ToolbarButton
                class="dock-tab-strip-tab"
                active={props.activeId === tab.id}
                title={tab.shortcut ? `${tab.label} (${tab.shortcut})` : tab.label}
                aria-label={tab.label}
                onClick={() => props.onTabChange(tab.id)}
              >
                <Icon size={15} stroke-width={1.75} />
              </ToolbarButton>
            );
          }}
        </For>
      </div>
      <Show when={props.onBack}>
        <DockPanelToolbarActions>
          <PanelHeaderAction
            title={props.backTitle ?? "Back"}
            aria-label={props.backTitle ?? "Back"}
            onClick={props.onBack}
          >
            <ChevronLeft size={15} />
          </PanelHeaderAction>
        </DockPanelToolbarActions>
      </Show>
    </DockPanelToolbar>
  );
}
