import { Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { credentialApi, credentialProviderFor, type AgentCredential } from "../../lib/tauriApi";
import { FieldRow, InlineSelect } from "./primitives";

const DEFAULT_VALUE = "";

/// Which account the role's CLI launches under. Saved immediately (like the MCP toggles) and
/// picked up on the role's next turn, since process slots compare launch env on reuse.
export function RoleAccountField(props: { roleId: string; runtimeKind: string }) {
  const provider = createMemo(() => credentialProviderFor(props.runtimeKind));
  const [accounts, setAccounts] = createSignal<AgentCredential[]>([]);
  const [selected, setSelected] = createSignal(DEFAULT_VALUE);
  const [error, setError] = createSignal("");

  createEffect(on([() => props.roleId, provider], ([roleId, key]) => {
    setError("");
    setSelected(DEFAULT_VALUE);
    if (!key) return;
    void Promise.all([credentialApi.overview(), credentialApi.roleBindings(roleId)])
      .then(([overview, bindings]) => {
        if (roleId !== props.roleId) return;
        setAccounts(overview.credentials.filter((c) => c.provider === key));
        setSelected(bindings.find((b) => b.provider === key)?.credentialId ?? DEFAULT_VALUE);
      })
      .catch((e) => setError(String(e)));
  }));

  const choose = async (value: string) => {
    const key = provider();
    if (!key) return;
    const previous = selected();
    setSelected(value);
    setError("");
    try {
      await credentialApi.setRoleBinding(props.roleId, key, value || null);
    } catch (e) {
      setSelected(previous);
      setError(String(e));
    }
  };

  const options = () => [
    { value: DEFAULT_VALUE, label: "Default (CLI login)" },
    ...accounts().map((a) => ({
      value: a.id,
      label: a.account.identity ? `${a.label} · ${a.account.identity}` : a.label,
    })),
  ];

  return (
    <Show when={provider()}>
      <FieldRow label="Account">
        <div class="flex w-full flex-col gap-1">
          <InlineSelect value={selected()} options={options()} onChange={(v) => void choose(v)} />
          <Show when={error()}>
            <span class="text-[10px] text-[var(--ui-state-danger-text)]">{error()}</span>
          </Show>
        </div>
      </FieldRow>
    </Show>
  );
}
