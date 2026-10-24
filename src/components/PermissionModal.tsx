import { For, Show, createSignal } from "solid-js";
import { canRememberChoice, isPlanApproval, isRejectOption, needsGenericDeny, planMarkdown, visiblePermissionOptions } from "../lib/permissionOptions";
import { renderMd } from "../lib/markdown";
import type { Accessor } from "solid-js";
import type { AppPermission, AppSession } from "./types";
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

/** One approval: title, the plan for a plan approval, and the agent's options. Shared by the
 *  conversation card, the tool row and the control center so they cannot drift apart. */
export function PermissionBody(props: {
  permission: AppPermission;
  disabled?: boolean;
  /** Compact surfaces (control center, phone) have no room for the Remember switch. */
  compact?: boolean;
  onChoose: (optionId: string) => void;
  onDeny: () => void;
}) {
  const [remember, setRemember] = createSignal(false);
  const plan = () => isPlanApproval(props.permission);
  const planText = () => (plan() ? planMarkdown(props.permission) : null);
  const opts = () => visiblePermissionOptions(props.permission.options, remember(), plan());
  const lead = () => opts().find((o) => !isRejectOption(o));
  return (
    <div class="permission-body">
      <div class="mb-1 text-xs font-semibold text-[var(--ui-state-warning-text)]">{props.permission.title}</div>
      <Show when={planText()}>
        {(text) => <div class="permission-plan md-prose" innerHTML={renderMd(text())} />}
      </Show>
      <Show when={props.permission.description}>
        <Show when={props.compact} fallback={<p class="mb-2 text-xs theme-muted">{props.permission.description}</p>}>
          <details class="mb-2 text-xs theme-muted"><summary class="cursor-pointer">Operation details</summary><p class="mt-2">{props.permission.description}</p></details>
        </Show>
      </Show>
      <div classList={{ "permission-option-list": plan(), "flex flex-wrap gap-2": !plan() }}>
        <For each={opts()}>{(opt) => (
          <Button
            // Rejecting is not destructive (nothing is lost), so it is outlined; filled red stays
            // reserved for deletes. The first allow option is the expected answer and leads.
            variant={isRejectOption(opt) ? "outline" : opt === lead() ? "primary" : "secondary"}
            size="sm"
            class={opt.description ? "permission-option has-description" : "permission-option"}
            disabled={props.disabled}
            onClick={(e) => { e.preventDefault(); props.onChoose(opt.optionId); }}
          >
            <span class="permission-option-label">{opt.title ?? opt.optionId}</span>
            <Show when={opt.description}><span class="permission-option-description">{opt.description}</span></Show>
          </Button>
        )}</For>
        <Show when={needsGenericDeny(props.permission.options)}>
          <Button
            variant="outline"
            size="sm"
            disabled={props.disabled}
            onClick={(e) => { e.preventDefault(); props.onDeny(); }}
          >
            Deny
          </Button>
        </Show>
      </div>
      <Show when={canRememberChoice(props.permission) && !props.compact}>
        <label class="mt-2 flex items-center gap-1.5 cursor-pointer select-none w-fit">
          <UiSwitch checked={remember()} onChange={setRemember} />
          <span class="text-[11px] theme-muted">Remember my choice</span>
        </label>
      </Show>
    </div>
  );
}

function PermissionsQueue(props: PermissionModalProps) {
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const queue = () => props.activeSession()?.pendingPermissions ?? [];
  const current = () => queue()[0]!;
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
      <PermissionBody
        permission={current()}
        compact={props.compact}
        disabled={submitting() || props.disabled}
        onChoose={(id) => void respond(id)}
        onDeny={() => void respond("", true)}
      />
    </Panel>
  );
}
