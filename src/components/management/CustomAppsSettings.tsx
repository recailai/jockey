import { For, Show, createSignal } from "solid-js";
import { Plus, Trash2 } from "lucide-solid";
import type { CustomWorkspaceApp } from "../../lib/uiPrefs";
import { BUILTIN_WORKSPACE_APPS } from "../../lib/workspaceApps";
import IdeAppIcon from "../IdeAppIcon";
import { Button, Input, Panel, PanelBody, RowButton } from "../ui";

type CustomAppsSettingsProps = {
  apps: CustomWorkspaceApp[];
  onChange: (apps: CustomWorkspaceApp[]) => void;
};

export default function CustomAppsSettings(props: CustomAppsSettingsProps) {
  const [draftLabel, setDraftLabel] = createSignal("");
  const [draftAppName, setDraftAppName] = createSignal("");
  const [draftBundleId, setDraftBundleId] = createSignal("");

  const addApp = () => {
    const label = draftLabel().trim();
    const appName = draftAppName().trim();
    if (!label || !appName) return;
    const bundleId = draftBundleId().trim();
    props.onChange([
      ...props.apps,
      {
        id: crypto.randomUUID(),
        label,
        appName,
        ...(bundleId ? { bundleId } : {}),
      },
    ]);
    setDraftLabel("");
    setDraftAppName("");
    setDraftBundleId("");
  };

  return (
    <>
      <div class="settings-section">
        <h2 class="settings-section-heading">Built-in apps</h2>
        <p class="settings-section-lead">Available in the composer “Open in” menu.</p>
        <Panel class="settings-card-list">
          <PanelBody class="settings-card-list-body">
            <For each={BUILTIN_WORKSPACE_APPS}>
              {(app) => (
                <div class="settings-custom-app-row is-readonly">
                  <IdeAppIcon target={app.target} />
                  <div class="min-w-0 flex-1">
                    <div class="text-[14px] font-medium theme-text">{app.label}</div>
                    <div class="truncate text-[12px] theme-muted">{app.appName}</div>
                  </div>
                </div>
              )}
            </For>
          </PanelBody>
        </Panel>
      </div>

      <div class="settings-section">
        <h2 class="settings-section-heading">Custom apps</h2>
        <p class="settings-section-lead">
          Add other macOS apps by their <code>.app</code> name (used with <code>open -a</code>).
          Bundle ID is optional and helps load the correct icon.
        </p>
        <Panel class="settings-card-list">
          <PanelBody class="settings-card-list-body">
            <Show when={props.apps.length > 0}>
              <For each={props.apps}>
                {(app) => (
                  <div class="settings-custom-app-row">
                    <IdeAppIcon target={`custom:${app.id}`} />
                    <div class="min-w-0 flex-1">
                      <div class="text-[14px] font-medium theme-text">{app.label}</div>
                      <div class="truncate text-[12px] theme-muted">{app.appName}</div>
                      <Show when={app.bundleId}>
                        <div class="truncate font-mono text-[10px] theme-muted">{app.bundleId}</div>
                      </Show>
                    </div>
                    <RowButton
                      class="settings-custom-app-remove"
                      title={`Remove ${app.label}`}
                      onClick={() => props.onChange(props.apps.filter((item) => item.id !== app.id))}
                    >
                      <Trash2 size={14} />
                    </RowButton>
                  </div>
                )}
              </For>
            </Show>
            <div class="settings-custom-app-form">
              <Input
                value={draftLabel()}
                onInput={(e) => setDraftLabel(e.currentTarget.value)}
                placeholder="Menu label"
                class="settings-custom-app-input"
              />
              <Input
                value={draftAppName()}
                onInput={(e) => setDraftAppName(e.currentTarget.value)}
                placeholder="App name (e.g. IntelliJ IDEA)"
                class="settings-custom-app-input"
              />
              <Input
                value={draftBundleId()}
                onInput={(e) => setDraftBundleId(e.currentTarget.value)}
                placeholder="Bundle ID (optional)"
                class="settings-custom-app-input"
              />
              <Button
                variant="default"
                size="md"
                class="settings-custom-app-add"
                disabled={!draftLabel().trim() || !draftAppName().trim()}
                onClick={addApp}
              >
                <Plus size={14} />
                Add app
              </Button>
            </div>
          </PanelBody>
        </Panel>
      </div>
    </>
  );
}
