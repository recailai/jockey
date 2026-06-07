import type { JSX } from "solid-js";
import { Show } from "solid-js";

type AppShellProps = {
  settings: JSX.Element;
  rightDock: JSX.Element;
  sessionTopbar: JSX.Element;
  conversation: JSX.Element;
  toasts: JSX.Element;
  showSettings: boolean;
};

export default function AppShell(props: AppShellProps) {
  return (
    <div class="jockey-app-shell window-bg h-dvh overflow-hidden text-[var(--ui-text)]">
      <Show when={props.showSettings} fallback={
        <div class="jockey-workbench">
          {props.sessionTopbar}
          <div class="jockey-workbench-body">
            <main class="jockey-main">
              {props.conversation}
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
