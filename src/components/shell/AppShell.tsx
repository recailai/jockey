import type { JSX } from "solid-js";
import { Show } from "solid-js";

type AppShellProps = {
  settings: JSX.Element;
  chrome: JSX.Element;
  sidebar: JSX.Element;
  header: JSX.Element;
  preview: JSX.Element;
  messages: JSX.Element;
  composer: JSX.Element;
  rightDock: JSX.Element;
  toasts: JSX.Element;
  showSettings: boolean;
};

export default function AppShell(props: AppShellProps) {
  return (
    <div
      class="jockey-app-shell window-bg h-dvh overflow-hidden text-[var(--ui-text)]"
      onContextMenu={(e) => e.preventDefault()}
    >
      <Show when={props.showSettings} fallback={
        <>
          {props.chrome}
          <div class="jockey-workbench">
            {props.sidebar}
            <main class="jockey-main">
              {props.header}
              <div class="jockey-main-body">
                <section class="jockey-chat-pane">
                  <div class="jockey-chat-stack">
                    {props.preview}
                    {props.messages}
                  </div>
                  {props.composer}
                </section>
                {props.rightDock}
              </div>
            </main>
          </div>
        </>
      }>
        {props.settings}
      </Show>
      {props.toasts}
    </div>
  );
}
