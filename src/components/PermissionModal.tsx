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
  return (
    <Show when={(props.activeSession()?.pendingPermissions ?? []).length > 0}>
      <PermissionsQueue activeSession={props.activeSession} patchActiveSession={props.patchActiveSession} />
    </Show>
  );
}

function PermissionsQueue(props: PermissionModalProps) {
  const [remember, setRemember] = createSignal(false);

  const queue = () => props.activeSession()?.pendingPermissions ?? [];
  const current = () => queue()[0]!;
  const hasRememberable = () => current().options.some((o) => o.kind === "allow_always");
  const opts = () => {
    const all = current().options;
    if (remember()) return all.filter((o) => o.kind === "allow_always" || !o.kind || o.kind === "allow_once");
    return all.filter((o) => o.kind !== "allow_always");
  };
  const advance = () => {
    props.patchActiveSession({ pendingPermissions: queue().slice(1) });
  };

  return (
    <Panel class="permission-panel my-2 px-3 py-2">
      <Show when={queue().length > 1}>
        <span class="mb-1 block text-[10px] font-mono theme-muted">
          permission 1 of {queue().length}
        </span>
      </Show>
      <div class="mb-1 text-xs font-semibold text-[var(--ui-state-warning-text)]">{current().title}</div>
      <Show when={current().description}><p class="mb-2 text-xs theme-muted">{current().description}</p></Show>
      <div class="flex flex-wrap gap-2">
        <For each={opts()}>{(opt) => (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              void assistantApi.respondPermission(current().requestId, opt.optionId, false);
              advance();
            }}
          >
            {opt.title ?? opt.optionId}
          </Button>
        )}</For>
        <Button
          variant="destructive"
          size="sm"
          onClick={() => {
            void assistantApi.respondPermission(current().requestId, "", true);
            advance();
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
}
