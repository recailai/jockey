import { For, Show, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession } from "./types";
import { assistantApi } from "../lib/tauriApi";
import { Button, Panel, Switch as UiSwitch } from "./ui";

type PermissionModalProps = {
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
};

export function PermissionModal(props: PermissionModalProps) {
  const [remember, setRemember] = createSignal(false);

  return (
    <Show when={props.activeSession()?.pendingPermission}>
      {(perm) => {
        const hasRememberable = () => perm().options.some((o) => o.kind === "allow_always");
        const opts = () => {
          const all = perm().options;
          if (remember()) return all.filter((o) => o.kind === "allow_always" || !o.kind || o.kind === "allow_once");
          return all.filter((o) => o.kind !== "allow_always");
        };
        return (
          <Panel class="permission-panel my-2 px-3 py-2">
            <div class="mb-1 text-xs font-semibold text-[var(--ui-state-warning-text)]">{perm().title}</div>
            <Show when={perm().description}><p class="mb-2 text-xs theme-muted">{perm().description}</p></Show>
            <div class="flex flex-wrap gap-2">
              <For each={opts()}>{(opt) => (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    void assistantApi.respondPermission(perm().requestId, opt.optionId, false);
                    props.patchActiveSession({ pendingPermission: null });
                  }}
                >
                  {opt.title ?? opt.optionId}
                </Button>
              )}</For>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => {
                  void assistantApi.respondPermission(perm().requestId, "", true);
                  props.patchActiveSession({ pendingPermission: null });
                }}
              >
                Deny
              </Button>
            </div>
            <Show when={hasRememberable()}>
              <label class="mt-2 flex items-center gap-1.5 cursor-pointer select-none w-fit">
                <UiSwitch
                  checked={remember()}
                  onChange={setRemember}
                />
                <span class="text-[11px] theme-muted">Remember my choice</span>
              </label>
            </Show>
          </Panel>
        );
      }}
    </Show>
  );
}
