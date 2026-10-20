import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { KeyRound, LogIn, Plus, RefreshCw, Trash2, X } from "lucide-solid";
import {
  credentialApi,
  type AgentCredential,
  type CredentialProviderInfo,
  type CredentialProviderKey,
  type CredentialsOverview,
} from "../../lib/tauriApi";
import { Alert, Badge, Button, FormField, IconButton, Input, Panel, PanelBody, Pill, createCopyFeedback } from "../ui";

type EnvRow = { key: string; value: string };

function AccountBadge(props: { signedIn: boolean; identity?: string | null }) {
  return (
    <Badge tone={props.signedIn ? "success" : "warning"}>
      {props.signedIn ? (props.identity || "signed in") : "not signed in"}
    </Badge>
  );
}

function EnvRowsEditor(props: {
  rows: EnvRow[];
  suggestions: string[];
  onChange: (rows: EnvRow[]) => void;
}) {
  const unused = () => props.suggestions.filter((key) => !props.rows.some((row) => row.key === key));
  const patch = (index: number, next: Partial<EnvRow>) =>
    props.onChange(props.rows.map((row, i) => (i === index ? { ...row, ...next } : row)));
  return (
    <div class="space-y-2">
      <For each={props.rows}>
        {(row, index) => (
          <div class="flex items-center gap-2">
            <Input
              value={row.key}
              placeholder="ENV_NAME"
              class="font-mono text-xs w-[220px]"
              onInput={(e) => patch(index(), { key: e.currentTarget.value.toUpperCase() })}
            />
            <Input
              type="password"
              value={row.value}
              placeholder="value"
              class="font-mono text-xs flex-1"
              onInput={(e) => patch(index(), { value: e.currentTarget.value })}
            />
            <IconButton size="sm" title="Remove" onClick={() => props.onChange(props.rows.filter((_, i) => i !== index()))}>
              <X size={13} />
            </IconButton>
          </div>
        )}
      </For>
      <div class="flex flex-wrap items-center gap-1.5">
        <Button variant="ghost" size="sm" onClick={() => props.onChange([...props.rows, { key: "", value: "" }])}>
          <Plus size={12} /> Env var
        </Button>
        <For each={unused()}>
          {(key) => (
            <Button variant="outline" size="sm" class="font-mono" onClick={() => props.onChange([...props.rows, { key, value: "" }])}>
              {key}
            </Button>
          )}
        </For>
      </div>
    </div>
  );
}

function envMap(rows: EnvRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (key && row.value) out[key] = row.value;
  }
  return out;
}

function AccountRow(props: {
  account: AgentCredential;
  provider: CredentialProviderInfo;
  busy: boolean;
  onUpdate: (input: { label?: string; setEnv?: Record<string, string>; removeEnv?: string[] }) => Promise<boolean>;
  onDelete: () => Promise<void>;
  onSignIn: () => Promise<void>;
}) {
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const [editingEnv, setEditingEnv] = createSignal(false);
  const [envRows, setEnvRows] = createSignal<EnvRow[]>([]);
  const copy = createCopyFeedback(() => props.account.loginCommand ?? "");

  const saveEnv = async () => {
    if (await props.onUpdate({ setEnv: envMap(envRows()) })) {
      setEnvRows([]);
      setEditingEnv(false);
    }
  };

  return (
    <div class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] p-3 space-y-2">
      <div class="flex items-center gap-2 flex-wrap">
        <span class="text-[13px] font-medium theme-text">{props.account.label}</span>
        <AccountBadge signedIn={props.account.account.signedIn} identity={props.account.account.identity} />
        <Show when={props.account.usedBy.length > 0}>
          <span class="text-[11px] theme-muted">used by {props.account.usedBy.join(", ")}</span>
        </Show>
        <div class="ml-auto flex items-center gap-1.5">
          <Show when={props.account.loginCommand}>
            <Button size="sm" variant="outline" disabled={props.busy} onClick={() => void props.onSignIn()}>
              <LogIn size={12} /> {props.account.account.signedIn ? "Switch account" : "Sign in"}
            </Button>
          </Show>
          <Show
            when={confirmDelete()}
            fallback={
              <IconButton size="sm" title="Delete account" disabled={props.busy} onClick={() => setConfirmDelete(true)}>
                <Trash2 size={13} />
              </IconButton>
            }
          >
            <Button size="sm" variant="danger" disabled={props.busy} onClick={() => void props.onDelete()}>
              Confirm delete
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>Cancel</Button>
          </Show>
        </div>
      </div>

      <Show when={props.account.configDir}>
        <div class="font-mono text-[11px] theme-muted break-all">{props.account.configDir}</div>
      </Show>
      <Show when={props.account.loginCommand}>
        {(command) => (
          <div class="flex items-center gap-2">
            <code class="font-mono text-[11px] theme-muted break-all flex-1">{command()}</code>
            <Button size="sm" variant="ghost" onClick={() => void copy.copy()}>
              {copy.copied() ? "Copied" : "Copy"}
            </Button>
          </div>
        )}
      </Show>

      <div class="flex flex-wrap items-center gap-1.5">
        <For each={props.account.envKeys}>
          {(key) => (
            <Pill class="font-mono text-[11px]">
              {key}
              <button
                type="button"
                class="ml-1 opacity-60 hover:opacity-100"
                title={`Remove ${key}`}
                disabled={props.busy}
                onClick={() => void props.onUpdate({ removeEnv: [key] })}
              >
                <X size={11} />
              </button>
            </Pill>
          )}
        </For>
        <Show when={!editingEnv()}>
          <Button size="sm" variant="ghost" onClick={() => { setEnvRows([{ key: "", value: "" }]); setEditingEnv(true); }}>
            <KeyRound size={12} /> Set env
          </Button>
        </Show>
      </div>
      <Show when={editingEnv()}>
        <div class="space-y-2">
          <EnvRowsEditor rows={envRows()} suggestions={props.provider.suggestedEnvKeys} onChange={setEnvRows} />
          <div class="flex gap-2">
            <Button size="sm" variant="primary" disabled={props.busy || Object.keys(envMap(envRows())).length === 0} onClick={() => void saveEnv()}>
              Save
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setEditingEnv(false)}>Cancel</Button>
          </div>
        </div>
      </Show>
    </div>
  );
}

function AddAccountForm(props: {
  provider: CredentialProviderInfo;
  busy: boolean;
  onCreate: (input: { label: string; configDir?: string | null; env: Record<string, string> }) => Promise<boolean>;
  onCancel: () => void;
}) {
  const [label, setLabel] = createSignal("");
  const [useExistingDir, setUseExistingDir] = createSignal(false);
  const [configDir, setConfigDir] = createSignal("");
  const [envRows, setEnvRows] = createSignal<EnvRow[]>(
    props.provider.supportsSignIn ? [] : [{ key: props.provider.suggestedEnvKeys[0] ?? "", value: "" }],
  );
  const canSubmit = () =>
    !!label().trim()
    && (!useExistingDir() || !!configDir().trim())
    && (props.provider.supportsSignIn || Object.keys(envMap(envRows())).length > 0);

  const submit = async () => {
    const ok = await props.onCreate({
      label: label().trim(),
      configDir: useExistingDir() ? configDir().trim() : null,
      env: envMap(envRows()),
    });
    if (ok) props.onCancel();
  };

  return (
    <div class="rounded-lg border border-[var(--ui-border-strong)] bg-[var(--ui-bg)] p-3 space-y-3">
      <FormField label="Account name">
        <Input value={label()} placeholder="e.g. Work" onInput={(e) => setLabel(e.currentTarget.value)} />
      </FormField>
      <Show when={props.provider.supportsSignIn}>
        <FormField
          label="Sign-in"
          description={useExistingDir()
            ? "Reuse a config dir that is already signed in (for example one you set up in a terminal)."
            : "Jockey creates a separate config dir; sign in to it after adding. Your current login is not touched."}
        >
          <div class="flex gap-2">
            <Button size="sm" variant={useExistingDir() ? "ghost" : "secondary"} onClick={() => setUseExistingDir(false)}>
              New sign-in
            </Button>
            <Button size="sm" variant={useExistingDir() ? "secondary" : "ghost"} onClick={() => setUseExistingDir(true)}>
              Existing config dir
            </Button>
          </div>
          <Show when={useExistingDir()}>
            <Input
              class="mt-2 font-mono text-xs"
              value={configDir()}
              placeholder="/Users/me/.claude-work"
              onInput={(e) => setConfigDir(e.currentTarget.value)}
            />
          </Show>
        </FormField>
      </Show>
      <FormField
        label={props.provider.supportsSignIn ? "Env (optional)" : "Env"}
        description={props.provider.supportsSignIn ? "Stored in the system keychain." : props.provider.loginHint}
      >
        <EnvRowsEditor rows={envRows()} suggestions={props.provider.suggestedEnvKeys} onChange={setEnvRows} />
      </FormField>
      <div class="flex gap-2">
        <Button size="sm" variant="primary" disabled={props.busy || !canSubmit()} onClick={() => void submit()}>
          Add account
        </Button>
        <Button size="sm" variant="ghost" onClick={props.onCancel}>Cancel</Button>
      </div>
    </div>
  );
}

export function AgentAccountsTab() {
  const [overview, setOverview] = createSignal<CredentialsOverview | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [adding, setAdding] = createSignal<CredentialProviderKey | null>(null);

  const accountsFor = (provider: CredentialProviderKey) =>
    (overview()?.credentials ?? []).filter((c) => c.provider === provider);
  const providers = createMemo(() => overview()?.providers ?? []);

  const run = async (action: () => Promise<CredentialsOverview | void>): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      const next = await action();
      if (next) setOverview(next);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const refresh = () => run(() => credentialApi.overview());

  onMount(() => { void refresh(); });

  return (
    <div class="flex flex-col gap-4 p-5">
      <div class="flex items-start justify-between gap-4">
        <p class="max-w-[640px] text-[13px] leading-relaxed theme-muted">
          Run roles under different agent accounts. Each CLI keeps its own login as the default; accounts added
          here live beside it, so adding or removing one never signs you out. Pick an account per role in the
          role editor.
        </p>
        <Button size="sm" variant="outline" disabled={busy()} onClick={() => void refresh()}>
          <RefreshCw size={12} classList={{ "animate-spin": busy() }} /> Refresh
        </Button>
      </div>

      <Show when={error()}>
        <Alert tone="danger" title="Account action failed" onClose={() => setError(null)}>
          {error()}
        </Alert>
      </Show>

      <For each={providers()}>
        {(provider) => (
          <Panel>
            <PanelBody class="space-y-3 p-4">
              <div class="flex items-center gap-2">
                <span class="text-[14px] font-medium theme-text">{provider.label}</span>
                <Button
                  size="sm"
                  variant="ghost"
                  class="ml-auto"
                  disabled={adding() === provider.key}
                  onClick={() => setAdding(provider.key)}
                >
                  <Plus size={12} /> Add account
                </Button>
              </div>

              <div class="flex items-center gap-2 flex-wrap rounded-lg border border-[var(--ui-border)] p-3">
                <span class="text-[13px] theme-text">Default</span>
                <AccountBadge signedIn={provider.defaultAccount.signedIn} identity={provider.defaultAccount.identity} />
                <span class="text-[11px] theme-muted">roles without an account use the CLI's own login</span>
              </div>

              <Show when={!provider.supportsSignIn}>
                <Alert tone="info">{provider.loginHint}</Alert>
              </Show>

              <For each={accountsFor(provider.key)}>
                {(account) => (
                  <AccountRow
                    account={account}
                    provider={provider}
                    busy={busy()}
                    onUpdate={(input) => run(() => credentialApi.update({ id: account.id, ...input }))}
                    onDelete={async () => { await run(() => credentialApi.remove(account.id)); }}
                    onSignIn={async () => { await run(() => credentialApi.openLogin(account.id)); }}
                  />
                )}
              </For>

              <Show when={adding() === provider.key}>
                <AddAccountForm
                  provider={provider}
                  busy={busy()}
                  onCreate={(input) => run(() => credentialApi.create({ provider: provider.key, ...input }))}
                  onCancel={() => setAdding(null)}
                />
              </Show>
            </PanelBody>
          </Panel>
        )}
      </For>
    </div>
  );
}
