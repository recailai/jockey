import type { JSX } from "solid-js";
import { Show } from "solid-js";
import { Button } from "../ui";

type AppShellProps = {
  leftSidebar?: JSX.Element;
  settings: JSX.Element;
  rightDock: JSX.Element;
  sessionTopbar: JSX.Element;
  conversation: JSX.Element;
  toasts: JSX.Element;
  showSettings: boolean;
  controlCenter?: JSX.Element;
  showControlCenter?: boolean;
  onToggleControlCenter?: () => void;
};

export default function AppShell(props: AppShellProps) {
  return (
    <div class="jockey-app-shell window-bg h-dvh overflow-hidden text-[var(--ui-text)]">
      <Show when={props.showSettings} fallback={
        <div class="jockey-workbench">
          {props.sessionTopbar}
          <div class="jockey-workbench-body">
            {props.leftSidebar}
            <main class="jockey-main">
              <Show when={props.onToggleControlCenter}>
                <div class="flex shrink-0 justify-end px-4 py-1"><Button variant="ghost" size="sm" onClick={props.onToggleControlCenter}>{props.showControlCenter ? "Back to chat" : "Control center"}</Button></div>
              </Show>
              <Show when={props.showControlCenter} fallback={props.conversation}>{props.controlCenter}</Show>
            </main>
            {props.rightDock}
          </div>
        </div>
      }>
        {props.settings}
      </Show>
      {props.toasts}
    </div>
  );
}
