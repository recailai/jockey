import type { JSX } from "solid-js";
import { Show } from "solid-js";

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
