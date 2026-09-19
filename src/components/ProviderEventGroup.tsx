import { For, Show } from "solid-js";
import { Badge } from "./ui";
import { isTransparentProviderEvent, type ProviderEventRecord } from "../lib/providerEventPolicy";

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function eventLabel(event: ProviderEventRecord): string {
  const payload = asRecord(event.payload);
  const nested = asRecord(payload?.event);
  const nestedType = typeof nested?.type === "string" ? nested.type : null;
  return nestedType && nestedType !== event.type ? `${event.type}/${nestedType}` : event.type;
}

function payloadText(payload: unknown): string {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
}

export function ProviderEventGroup(props: { events: ProviderEventRecord[] }) {
  const visibleEvents = () => props.events.filter((event) => !isTransparentProviderEvent(event));
  const labels = () => {
    const seen = new Set<string>();
    const values: string[] = [];
    for (const event of visibleEvents()) {
      const label = eventLabel(event);
      if (seen.has(label)) continue;
      seen.add(label);
      values.push(label);
    }
    const visible = values.slice(0, 4);
    const hidden = values.length - visible.length;
    return hidden > 0 ? `${visible.join(", ")}, +${hidden} more` : visible.join(", ");
  };

  return (
    <Show when={visibleEvents().length > 0}>
    <details class="provider-event-group">
      <summary class="provider-event-summary select-none">
        <Badge tone="neutral" variant="subtle">Provider events</Badge>
        <span class="theme-muted truncate" title={labels()}>{labels()}</span>
        <Badge class="ml-auto">{visibleEvents().length}</Badge>
        <svg class="provider-event-chevron theme-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </summary>
      <div class="provider-event-list">
        <For each={visibleEvents()}>{(event) => (
          <details class="provider-event-item">
            <summary class="provider-event-item-summary">
              <span class="ui-tool-status-dot ui-tool-status-info" />
              <span class="font-mono text-[10px] theme-text truncate">{eventLabel(event)}</span>
            </summary>
            <pre class="provider-event-payload">{payloadText(event.payload)}</pre>
          </details>
        )}</For>
      </div>
    </details>
    </Show>
  );
}
