import type { JSX } from "solid-js";

export type StateDotState = "done" | "warning" | "ongoing" | "error" | "idle";

export type StateDotProps = {
  state: StateDotState;
  /** Slot size; the dot itself is 60% of it so dots and the spinner line up in a row. */
  size?: number;
  class?: string;
};

export function StateDot(props: StateDotProps): JSX.Element {
  const size = () => props.size ?? 10;

  if (props.state === "ongoing") {
    return (
      <svg
        class={`jui-state-dot-spinner ${props.class ?? ""}`}
        data-state="ongoing"
        width={size()}
        height={size()}
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
      >
        <circle class="jui-state-dot-track" cx="12" cy="12" r="9" stroke="currentColor" stroke-width="3" />
        <circle class="jui-state-dot-arc" cx="12" cy="12" r="9" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-dasharray="12 150" />
      </svg>
    );
  }

  const dot = () => Math.max(4, Math.round(size() * 0.6));
  return (
    <span
      class={`inline-flex shrink-0 items-center justify-center ${props.class ?? ""}`}
      style={{ width: `${size()}px`, height: `${size()}px` }}
      aria-hidden="true"
    >
      <span class={`jui-state-dot jui-state-dot-${props.state}`} style={{ width: `${dot()}px`, height: `${dot()}px` }} />
    </span>
  );
}
