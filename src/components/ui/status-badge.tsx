import { Show } from "solid-js";
import type { JSX } from "solid-js";
import type { SessionStatus } from "../../lib/sessionStatus";
import { Badge, type BadgeTone } from "./badge";
import { StateDot, type StateDotState } from "./state-dot";

type StatusLook = { label: string; tone: BadgeTone; dot: StateDotState; hint: string };

export const SESSION_STATUS_LOOK: Record<SessionStatus, StatusLook> = {
  "needs-approval": { label: "Approval needed", tone: "warning", dot: "warning", hint: "A tool call is waiting for your decision" },
  "needs-answer": { label: "Answer needed", tone: "warning", dot: "warning", hint: "The agent asked you a question" },
  error: { label: "Agent error", tone: "danger", dot: "error", hint: "The last turn failed" },
  working: { label: "Working", tone: "info", dot: "ongoing", hint: "A turn is running" },
  external: { label: "Active in CLI", tone: "info", dot: "ongoing", hint: "Driven from a terminal outside Jockey" },
  idle: { label: "Idle", tone: "neutral", dot: "idle", hint: "Waiting for input" },
};

/** Dot only — for dense rows such as session tabs. Carries the label for assistive tech. */
export function StatusDot(props: { status: SessionStatus; size?: number; class?: string }): JSX.Element {
  const look = () => SESSION_STATUS_LOOK[props.status];
  return (
    <span class={`jui-status-dot ${props.class ?? ""}`} role="img" aria-label={look().label} title={look().hint}>
      <StateDot state={look().dot} size={props.size ?? 8} />
    </span>
  );
}

/** Dot plus label. `quiet` hides the idle state, which is the default and rarely worth a badge. */
export function StatusBadge(props: { status: SessionStatus; quiet?: boolean; label?: string; class?: string }): JSX.Element {
  const look = () => SESSION_STATUS_LOOK[props.status];
  return (
    <Show when={!(props.quiet && props.status === "idle")}>
      <Badge tone={look().tone} class={`jui-status-badge ${props.class ?? ""}`} title={look().hint}>
        <StateDot state={look().dot} size={7} />
        {props.label ?? look().label}
      </Badge>
    </Show>
  );
}
