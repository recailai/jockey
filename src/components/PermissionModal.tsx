import { For, Show, createSignal } from "solid-js";
import { isRejectOption, needsGenericDeny, visiblePermissionOptions } from "../lib/permissionOptions";
import type { Accessor } from "solid-js";
import type { AppSession } from "./types";
import { assistantApi } from "../lib/tauriApi";
import { Alert, Button, Panel, Switch as UiSwitch } from "./ui";

type PermissionModalProps = {
  activeSession: Accessor<AppSession | null>;
  patchActiveSession: (patch: Partial<AppSession>) => void;
  disabled?: boolean;
  onSubmittingChange?: (submitting: boolean) => void;
  compact?: boolean;
};

export function PermissionModal(props: PermissionModalProps) {
  return (
    <Show when={(props.activeSession()?.pendingPermissions ?? []).length > 0}>
      <PermissionsQueue {...props} />
    </Show>
  );
}

function PermissionsQueue(props: PermissionModalProps) {
  const [remember, setRemember] = createSignal(false);
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const queue = () => props.activeSession()?.pendingPermissions ?? [];
  const current = () => queue()[0]!;
  const hasRememberable = () => current().options.some((o) => (o.kind ?? "").endsWith("_always"));
  const opts = () => {
    const all = current().options;
    return visiblePermissionOptions(all, remember());
  };
  const respond = async (optionId: string, cancelled = false) => {
    if (submitting() || props.disabled) return;
    const requestId = current().requestId;
    const sid = props.activeSession()?.id;
    setSubmitting(true);
    props.onSubmittingChange?.(true);
    setError(null);
    try {
      await assistantApi.respondPermission(requestId, optionId, cancelled);
      if (props.activeSession()?.id === sid) {
        props.patchActiveSession({ pendingPermissions: queue().filter((p) => p.requestId !== requestId) });
      }
    } catch (e) {
      setError(`Could not submit decision: ${String(e)}`);
    } finally {
      setSubmitting(false);
      props.onSubmittingChange?.(false);
    }
  };

  return (
    <Panel class="permission-panel my-2 px-3 py-2">
      <Show when={error()}><Alert tone="danger">{error()}</Alert></Show>
      <Show when={queue().length > 1}>
        <span class="mb-1 block text-[10px] font-mono theme-muted">
          permission 1 of {queue().length}
        </span>
      </Show>
      <div class="mb-1 text-xs font-semibold text-[var(--ui-state-warning-text)]">{current().title}</div>
      <Show when={current().description}>
        <Show when={props.compact} fallback={<p class="mb-2 text-xs theme-muted">{current().description}</p>}>
          <details class="mb-2 text-xs theme-muted"><summary class="cursor-pointer">Operation details</summary><p class="mt-2">{current().description}</p></details>
        </Show>
      </Show>
      <div class="flex flex-wrap gap-2">
        <For each={opts()}>{(opt) => (
          <Button
            variant={isRejectOption(opt) ? "destructive" : "secondary"}
            size="sm"
            disabled={submitting() || props.disabled}
            onClick={() => void respond(opt.optionId)}
          >
            {opt.title ?? opt.optionId}
          </Button>
        )}</For>
        <Show when={needsGenericDeny(current().options)}>
          <Button
            variant="destructive"
            size="sm"
            disabled={submitting() || props.disabled}
            onClick={() => void respond("", true)}
          >
            Deny
          </Button>
        </Show>
      </div>
      <Show when={hasRememberable() && !props.compact}>
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
