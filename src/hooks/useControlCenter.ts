import { createSignal, onCleanup, onMount } from "solid-js";
import type { SessionSummary } from "../components/types";
import { assistantApi } from "../lib/tauriApi";
import { call, listen, type UnlistenFn } from "../lib/transport";

export type PendingDecisions = Awaited<ReturnType<typeof assistantApi.listPendingPrompts>>;

export function useControlCenter() {
  const [sessions, setSessions] = createSignal<SessionSummary[]>([]);
  const [pending, setPending] = createSignal<PendingDecisions>({ permissions: [], userInputs: [] });
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let disposed = false;
  let revision = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const refresh = async () => {
    const current = ++revision;
    try {
      const [rows, decisions] = await Promise.all([
        call<SessionSummary[]>("list_session_summaries_cmd"),
        assistantApi.listPendingPrompts(),
      ]);
      if (disposed || current !== revision) return;
      setSessions(rows);
      setPending(decisions);
      setError(null);
    } catch (e) {
      if (!disposed && current === revision) setError(String(e));
    } finally {
      if (!disposed && current === revision) setLoading(false);
    }
  };
  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(() => void refresh(), 200);
  };

  onMount(() => {
    void refresh();
    const handlers: UnlistenFn[] = [];
    for (const event of ["turn/started", "turn/finished", "queue/changed", "session/changed", "project/changed", "permission_update"]) {
      void listen(event, schedule).then((off) => disposed ? off() : handlers.push(off)).catch((e) => setError(String(e)));
    }
    void listen<{ event?: { kind?: string } }>("acp/stream", ({ payload }) => {
      if (["permissionRequest", "permissionExpired", "userInputRequest"].includes(payload.event?.kind ?? "")) schedule();
    }).then((off) => disposed ? off() : handlers.push(off)).catch((e) => setError(String(e)));
    const visible = () => { if (document.visibilityState === "visible") void refresh(); };
    document.addEventListener("visibilitychange", visible);
    const poll = setInterval(visible, 15_000);
    onCleanup(() => {
      disposed = true;
      clearTimeout(timer);
      clearInterval(poll);
      handlers.forEach((off) => off());
      document.removeEventListener("visibilitychange", visible);
    });
  });
  return { sessions, pending, loading, error, refresh };
}
