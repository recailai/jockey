import { For, Show, createSignal } from "solid-js";
import { Plus, X } from "lucide-solid";
import {
  llmProviderApi,
  type LiveLlmProvider,
  type SavedLlmProviderConfig,
} from "../../lib/tauriApi";
import { Alert, Button, FormField, IconButton, Input } from "../ui";

type HeaderRow = { name: string; value: string; stored: boolean };

function ProviderForm(props: {
  runtimeKey: string;
  provider: LiveLlmProvider;
  saved?: SavedLlmProviderConfig;
  applyError?: string | null;
  onChanged: () => Promise<void>;
}) {
  const initialProtocol = () =>
    props.saved?.apiType ?? props.provider.currentApiType ?? props.provider.supported[0] ?? "";
  const [apiType, setApiType] = createSignal(initialProtocol());
  const [baseUrl, setBaseUrl] = createSignal(props.saved?.baseUrl ?? props.provider.currentBaseUrl ?? "");
  const [rows, setRows] = createSignal<HeaderRow[]>(
    (props.saved?.headerNames ?? []).map((name) => ({ name, value: "", stored: true })),
  );
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const patch = (index: number, next: Partial<HeaderRow>) =>
    setRows(rows().map((row, i) => (i === index ? { ...row, ...next } : row)));

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await props.onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(() =>
      llmProviderApi.save(props.runtimeKey, {
        providerId: props.provider.providerId,
        apiType: apiType(),
        baseUrl: baseUrl().trim(),
        headers: rows()
          .filter((row) => row.name.trim())
          .map((row) => ({ name: row.name.trim(), value: row.value || undefined })),
      }),
    );

  const clear = () => run(() => llmProviderApi.clear(props.runtimeKey, props.provider.providerId));

  return (
    <div class="rounded-lg border border-[var(--ui-border)] bg-[var(--ui-bg)] p-3 space-y-3">
      <span class="font-mono text-[12px] theme-text">{props.provider.providerId}</span>
      <FormField label="Protocol">
        <div class="flex flex-wrap gap-2">
          <For each={props.provider.supported}>
            {(protocol) => (
              <Button
                size="sm"
                variant={apiType() === protocol ? "secondary" : "ghost"}
                onClick={() => setApiType(protocol)}
              >
                {protocol}
              </Button>
            )}
          </For>
        </div>
      </FormField>
      <FormField label="Base URL">
        <Input
          monospace
          value={baseUrl()}
          placeholder="https://llm.example.com"
          onInput={(e) => setBaseUrl(e.currentTarget.value)}
        />
      </FormField>
      <FormField label="Headers">
        <div class="space-y-2">
          <For each={rows()}>
            {(row, index) => (
              <div class="flex items-center gap-2">
                <Input
                  monospace
                  class="w-[200px] text-xs"
                  value={row.name}
                  placeholder="Authorization"
                  onInput={(e) => patch(index(), { name: e.currentTarget.value })}
                />
                <Input
                  type="password"
                  monospace
                  class="flex-1 text-xs"
                  autocomplete="off"
                  value={row.value}
                  placeholder={row.stored ? "••••••••" : "value"}
                  onInput={(e) => patch(index(), { value: e.currentTarget.value })}
                />
                <IconButton size="sm" title="Remove header" onClick={() => setRows(rows().filter((_, i) => i !== index()))}>
                  <X size={13} />
                </IconButton>
              </div>
            )}
          </For>
          <Button size="sm" variant="ghost" onClick={() => setRows([...rows(), { name: "", value: "", stored: false }])}>
            <Plus size={12} /> Header
          </Button>
        </div>
      </FormField>
      <Show when={props.applyError}>
        <Alert tone="warning" title="Last connection ignored this endpoint">{props.applyError}</Alert>
      </Show>
      <Show when={error()}>
        <Alert tone="danger" onClose={() => setError(null)}>{error()}</Alert>
      </Show>
      <div class="flex gap-2">
        <Button size="sm" variant="primary" disabled={busy() || !apiType() || !baseUrl().trim()} onClick={() => void save()}>
          Save
        </Button>
        <Show when={props.saved}>
          <Button size="sm" variant="ghost" disabled={busy()} onClick={() => void clear()}>
            Clear
          </Button>
        </Show>
      </div>
    </div>
  );
}

/// Endpoint routing for one runtime. Providers come live from the adapter, which costs a
/// short-lived process, so they load on demand; nothing renders for adapters that do not
/// advertise `providers`.
export function LlmEndpointSection(props: { runtimeKey: string }) {
  const [providers, setProviders] = createSignal<LiveLlmProvider[] | null>(null);
  const [saved, setSaved] = createSignal<SavedLlmProviderConfig[]>([]);
  const [applyError, setApplyError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [unsupported, setUnsupported] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [open, setOpen] = createSignal(false);

  const refreshSaved = async () => {
    const s = await llmProviderApi.saved(props.runtimeKey);
    setSaved(s.configs);
    setApplyError(s.applyError ?? null);
  };

  const expand = async () => {
    setOpen(true);
    setLoading(true);
    setError(null);
    try {
      await refreshSaved();
      const live = await llmProviderApi.live(props.runtimeKey);
      setProviders(live);
      setUnsupported(live === null || live.length === 0);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <Show when={!unsupported()}>
      <div class="space-y-2">
        <Show
          when={open()}
          fallback={
            <Button size="sm" variant="ghost" onClick={() => void expand()}>
              Model endpoint
            </Button>
          }
        >
          <span class="text-[13px] theme-text">Model endpoint</span>
          <Show when={loading()}>
            <span class="text-[12px] theme-muted ml-2">Loading…</span>
          </Show>
          <Show when={error()}>
            <Alert tone="danger" onClose={() => setError(null)}>{error()}</Alert>
          </Show>
          <For each={providers() ?? []}>
            {(provider) => (
              <ProviderForm
                runtimeKey={props.runtimeKey}
                provider={provider}
                saved={saved().find((c) => c.providerId === provider.providerId)}
                applyError={applyError()}
                onChanged={refreshSaved}
              />
            )}
          </For>
        </Show>
      </div>
    </Show>
  );
}
