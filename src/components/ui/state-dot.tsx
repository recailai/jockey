import { For } from "solid-js";
import type { JSX } from "solid-js";

export type StateDotState = "done" | "warning" | "ongoing" | "error" | "idle";

const MATRIX_CELLS: readonly (readonly [number, number])[] = [
  [0, 0], [4, 0], [8, 0], [8, 4], [8, 8], [4, 8], [0, 8], [0, 4],
];

export type StateDotProps = {
  state: StateDotState;
  size?: number;
  class?: string;
};

export function StateDot(props: StateDotProps): JSX.Element {
  const size = () => props.size ?? 10;

  if (props.state === "ongoing") {
    return (
      <svg
        class={`jui-state-dot-matrix ${props.class ?? ""}`}
        data-state="ongoing"
        width={size()}
        height={size()}
        viewBox="0 0 10 10"
        shape-rendering="crispEdges"
        aria-hidden="true"
      >
        <For each={MATRIX_CELLS}>
          {([x, y], index) => (
            <rect
              class="jui-state-dot-cell"
              x={x}
              y={y}
              width="2"
              height="2"
              style={{
                "animation-delay": `${(index() - MATRIX_CELLS.length) * 125}ms`,
              }}
            />
          )}
        </For>
      </svg>
    );
  }

  return (
    <span
      class={`jui-state-dot jui-state-dot-${props.state} ${props.class ?? ""}`}
      style={{ width: `${size()}px`, height: `${size()}px` }}
      aria-hidden="true"
    />
  );
}
