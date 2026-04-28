import { Show } from "solid-js";
import type { AppSession, SessionErrorInfo } from "./types";
import { assistantApi } from "../lib/tauriApi";
import { Badge, Button } from "./ui";

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
      return "is-warning";
    case "connection":
      return "is-danger";
    case "timeout":
      return "is-info";
    default:
      return "is-muted";
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
            class={`session-error-banner ${toneClasses(tone)}`}
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
                <Badge tone="warning">Re-auth via CLI</Badge>
              </Show>
              <Show when={isConn}>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={reconnect}
                >
                  Reconnect
                </Button>
              </Show>
              <Show when={isTimeout}>
                <Badge tone="info">Retry next turn</Badge>
              </Show>
              <Button
                variant="ghost"
                size="sm"
                onClick={dismiss}
                title="Dismiss"
              >
                Dismiss
              </Button>
            </div>
          </div>
        );
      }}
    </Show>
  );
}
