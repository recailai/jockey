import { For, Show, createSignal } from "solid-js";
import type { Accessor } from "solid-js";
import type { AppSession } from "./types";
import { INTERACTIVE_MOTION } from "./types";
import { assistantApi } from "../lib/tauriApi";

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
          <div class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 my-2">
            <div class="mb-1 text-xs font-semibold text-amber-300">{perm().title}</div>
            <Show when={perm().description}><p class="mb-2 text-xs text-zinc-400">{perm().description}</p></Show>
            <div class="flex flex-wrap gap-2">
              <For each={opts()}>{(opt) => (
                <button
                  class={`min-h-8 rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
                  onClick={() => {
                    void assistantApi.respondPermission(perm().requestId, opt.optionId, false);
                    props.patchActiveSession({ pendingPermission: null });
                  }}
                >
                  {opt.title ?? opt.optionId}
                </button>
              )}</For>
              <button
                class={`min-h-8 rounded border border-rose-500/40 bg-rose-500/10 px-3 py-1 text-xs text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
                onClick={() => {
                  void assistantApi.respondPermission(perm().requestId, "", true);
                  props.patchActiveSession({ pendingPermission: null });
                }}
              >
                Deny
              </button>
            </div>
            <Show when={hasRememberable()}>
              <label class="mt-2 flex items-center gap-1.5 cursor-pointer select-none w-fit">
                <input
                  type="checkbox"
                  checked={remember()}
                  onChange={(e) => setRemember(e.currentTarget.checked)}
                  class="accent-amber-400 h-3 w-3"
                />
                <span class="text-[10px] text-zinc-400">Remember my choice</span>
              </label>
            </Show>
          </div>
        );
      }}
    </Show>
  );
}
