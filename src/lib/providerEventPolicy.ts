export type ProviderEventRecord = {
  type: string;
  payload: unknown;
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function nestedTypes(payload: unknown): string[] {
  const record = asRecord(payload);
  const nested = asRecord(record?.event);
  const values = [record?.type, record?.method, record?.update, nested?.type];
  return values.filter((value): value is string => typeof value === "string");
}

const TRANSPARENT_PROVIDER_EVENTS = new Set([
  "account/rateLimits/updated",
  "turn/diff/updated",
  "thread/started",
  "thread/resumed",
  "thread/archived",
  "turn/started",
  "stream_event/message_start",
  "stream_event/message_stop",
  "stream_event/ping",
]);

export function isTransparentProviderEvent(event: ProviderEventRecord): boolean {
  if (TRANSPARENT_PROVIDER_EVENTS.has(event.type)) return true;

  return nestedTypes(event.payload).some((type) => {
    if (TRANSPARENT_PROVIDER_EVENTS.has(type)) return true;
    return event.type === "stream_event" && TRANSPARENT_PROVIDER_EVENTS.has(`stream_event/${type}`);
  });
}
