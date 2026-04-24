import { Show } from "solid-js";
import type { AppSession, SessionErrorInfo } from "./types";
import { assistantApi } from "../lib/tauriApi";

type Props = {
  activeSession: () => AppSession | null;
  activeSessionId: () => string | null;
  activeBackendRole: () => string;
  patchActiveSession: (patch: Partial<AppSession>) => void;
};

type Tone = "auth" | "connection" | "timeout" | "generic";

function toneFor(code: string): Tone {
  if (code === "AUTH_REQUIRED") return "auth";
  if (code === "CONNECTION_FAILED" || code === "PROCESS_CRASHED") return "connection";
  if (code === "PROMPT_TIMEOUT") return "timeout";
  return "generic";
}

function toneClasses(tone: Tone): string {
  switch (tone) {
    case "auth":
      return "border-amber-500/30 bg-amber-500/[0.08] text-amber-200";
    case "connection":
      return "border-rose-500/30 bg-rose-500/[0.08] text-rose-200";
    case "timeout":
      return "border-indigo-500/30 bg-indigo-500/[0.08] text-indigo-200";
    default:
      return "border-[var(--ui-border-strong)] bg-[var(--ui-surface-muted)] theme-text";
  }
}

function label(code: string): string {
  switch (code) {
    case "AUTH_REQUIRED":
      return "Authentication required";
    case "CONNECTION_FAILED":
      return "Connection failed";
    case "PROCESS_CRASHED":
      return "Agent process crashed";
    case "PROMPT_TIMEOUT":
      return "Prompt timeout";
    case "ACP_REQ_CANCELLED":
      return "Request cancelled";
    case "AGENT_ERROR":
      return "Agent error";
    case "UNSUPPORTED_CONFIG":
    case "ACP_INVALID_PARAMS":
      return "Unsupported configuration";
    default:
      return code.replace(/_/g, " ").toLowerCase();
  }
}

export default function SessionErrorBanner(props: Props) {
  const err = (): SessionErrorInfo | null => props.activeSession()?.lastError ?? null;
  const dismiss = () => props.patchActiveSession({ lastError: null });

  const reconnect = async () => {
    const sid = props.activeSessionId();
    if (!sid) return;
    try {
      await assistantApi.reconnectSession(props.activeBackendRole(), sid);
      dismiss();
    } catch {
      // swallow; reconnect itself surfaces a fresh error
    }
  };

  return (
    <Show when={err()}>
      {(e) => {
        const tone = toneFor(e().code);
        const isAuth = tone === "auth";
        const isConn = tone === "connection";
        const isTimeout = tone === "timeout";
        return (
          <div
            class={`flex items-start gap-2 rounded-lg border px-3 py-2 my-2 text-[11.5px] ${toneClasses(tone)}`}
            role="alert"
          >
            <div class="flex-1 min-w-0">
              <div class="font-semibold tracking-wide">
                {label(e().code)}
                <Show when={e().retryable}>
                  <span class="ml-1.5 text-[9px] font-bold uppercase tracking-widest opacity-70">
                    retryable
                  </span>
                </Show>
              </div>
              <div class="mt-0.5 text-[11px] font-mono break-words opacity-90">{e().message}</div>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Show when={isAuth}>
                <span class="rounded-md border border-amber-500/40 bg-amber-500/15 px-2 py-[2px] text-[10px] font-medium uppercase tracking-wider">
                  Re-auth via CLI
                </span>
              </Show>
              <Show when={isConn}>
                <button
                  onClick={reconnect}
                  class="rounded-md border border-rose-500/40 bg-rose-500/15 px-2 py-[2px] text-[10px] font-medium uppercase tracking-wider hover:bg-rose-500/25 transition-colors"
                >
                  Reconnect
                </button>
              </Show>
              <Show when={isTimeout}>
                <span class="rounded-md border border-indigo-500/40 bg-indigo-500/15 px-2 py-[2px] text-[10px] font-medium uppercase tracking-wider">
                  Retry next turn
                </span>
              </Show>
              <button
                onClick={dismiss}
                class="rounded-md theme-muted hover:text-primary hover:bg-white/10 transition-colors px-2 py-[2px] text-[10px] font-medium uppercase tracking-wider"
                title="Dismiss"
              >
                Dismiss
              </button>
            </div>
          </div>
        );
      }}
    </Show>
  );
}
