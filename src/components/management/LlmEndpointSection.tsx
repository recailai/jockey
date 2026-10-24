import { For, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { ChevronDown, Plus, X } from "lucide-solid";
import {
  llmProviderApi,
  type LiveLlmProvider,
  type SavedLlmProviderConfig,
} from "../../lib/tauriApi";
import { Alert, Button, FormField, IconButton, Input, Panel, PanelBody } from "../ui";

type HeaderRow = { name: string; value: string; stored: boolean };
type Notify = (message: string, severity?: "error" | "info") => void;

const storedRows = (saved?: SavedLlmProviderConfig): HeaderRow[] =>
  (saved?.headerNames ?? []).map((name) => ({ name, value: "", stored: true }));

/** The protocol to preselect. A saved protocol the adapter no longer offers is left unselected
 *  rather than shown as if it were valid. */
const initialProtocol = (provider: LiveLlmProvider, saved?: SavedLlmProviderConfig): string => {
  if (saved) return provider.supported.includes(saved.apiType) ? saved.apiType : "";
  const current = provider.currentApiType;
  return current && provider.supported.includes(current) ? current : provider.supported[0] ?? "";
};

function ProviderForm(props: {
  runtimeKey: string;
  provider: LiveLlmProvider;
  saved?: SavedLlmProviderConfig;
  showName: boolean;
  showToast?: Notify;
  onChanged: () => Promise<void>;
}) {
  const [apiType, setApiType] = createSignal(initialProtocol(props.provider, props.saved));
  const [baseUrl, setBaseUrl] = createSignal(props.saved?.baseUrl ?? props.provider.currentBaseUrl ?? "");
  const [rows, setRows] = createSignal<HeaderRow[]>(storedRows(props.saved));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // The form mirrors what is stored: after Save the typed secrets are gone and their rows are
  // marked stored; after Clear nothing is left to show.
  createEffect(
    on(
      () => JSON.stringify(props.saved ?? null),
      () => {
        setApiType(initialProtocol(props.provider, props.saved));
        setBaseUrl(props.saved ? props.saved.baseUrl : "");
        setRows(storedRows(props.saved));
      },
      { defer: true },
    ),
  );

  const protocolMissing = () => !!props.saved && !props.provider.supported.includes(props.saved.apiType);

  const duplicateName = createMemo(() => {
    const seen = new Set<string>();
    for (const row of rows()) {
      const name = row.name.trim().toLowerCase();
      if (!name) continue;
      if (seen.has(name)) return row.name.trim();
      seen.add(name);
    }
    return null;
  });

  const patch = (index: number, next: Partial<HeaderRow>) =>
    setRows(rows().map((row, i) => (i === index ? { ...row, ...next } : row)));

  const run = async (action: () => Promise<void>, done: string) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await props.onChanged();
      props.showToast?.(done, "info");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(
      () =>
        llmProviderApi.save(props.runtimeKey, {
          providerId: props.provider.providerId,
          apiType: apiType(),
          baseUrl: baseUrl().trim(),
          headers: rows()
            .filter((row) => row.name.trim())
            .map((row) => ({ name: row.name.trim(), value: row.value || undefined })),
        }),
      "Endpoint saved",
    ).then(() => {
      if (!error()) {
        setRows(rows().filter((r) => r.name.trim()).map((r) => ({ name: r.name.trim(), value: "", stored: true })));
      }
    });

  const clear = () =>
    run(() => llmProviderApi.clear(props.runtimeKey, props.provider.providerId), "Endpoint cleared");

  return (
    <Panel>
      <PanelBody class="space-y-3 p-3">
        <Show when={props.showName}>
          <span class="font-mono text-[12px] theme-text">{props.provider.providerId}</span>
        </Show>
        <FormField
          label="Protocol"
          error={protocolMissing() ? `"${props.saved?.apiType}" is no longer supported; pick another.` : undefined}
        >
          <div class="flex flex-wrap gap-2" role="group" aria-label="Protocol">
            <For each={props.provider.supported}>
              {(protocol) => (
                <Button
                  size="sm"
                  variant={apiType() === protocol ? "secondary" : "ghost"}
                  aria-pressed={apiType() === protocol}
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
        <FormField label="Headers" error={duplicateName() ? `Header "${duplicateName()}" is listed twice.` : undefined}>
          <div class="space-y-2">
            <For each={rows()}>
              {(row, index) => (
                <div class="flex flex-wrap items-center gap-2">
                  <Input
                    monospace
                    class="min-w-0 flex-[1_1_140px] text-xs"
                    aria-label="Header name"
                    value={row.name}
                    placeholder="Authorization"
                    onInput={(e) => patch(index(), { name: e.currentTarget.value })}
                  />
                  <Input
                    type="password"
                    monospace
                    class="min-w-0 flex-[2_1_160px] text-xs"
                    aria-label="Header value"
                    autocomplete="off"
                    value={row.value}
                    placeholder={row.stored ? "••••••••" : "value"}
                    onInput={(e) => patch(index(), { value: e.currentTarget.value })}
                  />
                  <IconButton
                    size="sm"
                    title="Remove header"
                    aria-label={`Remove header ${row.name || index() + 1}`}
                    onClick={() => setRows(rows().filter((_, i) => i !== index()))}
                  >
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
        <Show when={error()}>
          <Alert tone="danger" onClose={() => setError(null)}>{error()}</Alert>
        </Show>
        <div class="flex gap-2">
          <Button
            size="sm"
            variant="primary"
            disabled={busy() || !apiType() || !baseUrl().trim() || !!duplicateName()}
            onClick={() => void save()}
          >
            Save
          </Button>
          <Show when={props.saved}>
            <Button size="sm" variant="ghost" disabled={busy()} onClick={() => void clear()}>
              Clear
            </Button>
          </Show>
        </div>
      </PanelBody>
    </Panel>
  );
}

/// Endpoint routing for one runtime. Providers come live from the adapter, which costs a
/// short-lived process, so they load on first open.
export function LlmEndpointSection(props: { runtimeKey: string; showToast?: Notify }) {
  const [providers, setProviders] = createSignal<LiveLlmProvider[] | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [saved, setSaved] = createSignal<SavedLlmProviderConfig[]>([]);
  const [applyError, setApplyError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [open, setOpen] = createSignal(false);

  const refreshSaved = async () => {
    const s = await llmProviderApi.saved(props.runtimeKey);
    setSaved(s.configs);
    setApplyError(s.applyError ?? null);
  };

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      await refreshSaved();
      setProviders(await llmProviderApi.live(props.runtimeKey));
      setLoaded(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const toggle = () => {
    const next = !open();
    setOpen(next);
    if (next && !loaded() && !loading()) void load();
  };

  const unsupported = () => loaded() && (providers()?.length ?? 0) === 0;

  return (
    <div class="space-y-2">
      <Button size="sm" variant="ghost" aria-expanded={open()} onClick={toggle}>
        Model endpoint
        <ChevronDown size={12} class="transition-transform" classList={{ "rotate-180": open() }} />
      </Button>
      <Show when={open()}>
        <Show when={loading()}>
          <span class="text-[12px] theme-muted">Loading…</span>
        </Show>
        <Show when={error()}>
          <Alert tone="danger" action={{ label: "Retry", onClick: () => void load() }}>{error()}</Alert>
        </Show>
        <Show when={unsupported()}>
          <span class="text-[12px] theme-muted">Not supported</span>
        </Show>
        <Show when={applyError()}>
          <Alert tone="warning" title="Last connection ignored this endpoint">{applyError()}</Alert>
        </Show>
        <For each={providers() ?? []}>
          {(provider) => (
            <ProviderForm
              runtimeKey={props.runtimeKey}
              provider={provider}
              saved={saved().find((c) => c.providerId === provider.providerId)}
              showName={(providers()?.length ?? 0) > 1}
              showToast={props.showToast}
              onChanged={refreshSaved}
            />
          )}
        </For>
      </Show>
    </div>
  );
}
