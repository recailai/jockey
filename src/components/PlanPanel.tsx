import { For, Show } from "solid-js";
import { ChevronDown } from "lucide-solid";
import type { AppPlanEntry } from "./types";
import { Badge, Panel, Pill, StateDot } from "./ui";
import { renderMd } from "../lib/markdown";

const stepText = (entry: AppPlanEntry) => entry.content ?? entry.title ?? entry.description ?? "step";

/** The agent's plan: a checklist of steps, or a single document when the plan was sent whole. */
export function PlanPanel(props: { plan: AppPlanEntry[] }) {
  const total = () => props.plan.length;
  const done = () => props.plan.filter((e) => e.status === "completed").length;
  const running = () => props.plan.some((e) => e.status === "in_progress");
  const allDone = () => done() === total() && total() > 0;
  const doc = () => (props.plan.length === 1 && props.plan[0].format ? props.plan[0] : null);

  return (
    <Panel class="plan-panel my-2 overflow-hidden">
      <details class="group/plan">
        <summary class="plan-panel-summary">
          <StateDot state={running() ? "ongoing" : allDone() ? "done" : "idle"} size={11} />
          <span class="theme-text font-semibold">Plan</span>
          <Show when={running()}><Badge tone="warning">running</Badge></Show>
          <Show when={allDone()}><Badge tone="success">done</Badge></Show>
          <span class="ml-auto" />
          <Show when={!doc()}>
            <Pill class="font-mono">{done()}/{total()}</Pill>
          </Show>
          <ChevronDown size={13} class="theme-muted transition-transform duration-150 group-open/plan:rotate-180" />
        </summary>
        <Show
          when={doc()}
          fallback={
            <div class="border-t theme-border">
              <For each={props.plan}>{(entry, i) => {
                const isRunning = () => entry.status === "in_progress";
                const isCompleted = () => entry.status === "completed";
                return (
                  <div
                    class="flex items-start gap-2.5 px-3 py-1.5 text-[12px] border-b theme-border last:border-b-0"
                    classList={{ "plan-step-running": isRunning() }}
                  >
                    <span class="mt-[2px] w-4 shrink-0 text-right font-mono text-[11px] theme-muted">{i() + 1}</span>
                    <span
                      class="min-w-0 flex-1 break-words leading-relaxed"
                      classList={{
                        "theme-text": !isCompleted(),
                        "theme-muted line-through": isCompleted(),
                      }}
                    >
                      {stepText(entry)}
                    </span>
                    <Show when={entry.priority === "high" || entry.priority === "medium"}>
                      <Badge tone={entry.priority === "high" ? "danger" : "warning"}>{entry.priority}</Badge>
                    </Show>
                  </div>
                );
              }}</For>
            </div>
          }
        >
          {(d) => (
            <div class="plan-panel-doc border-t theme-border px-3 py-2 text-[12px]">
              <Show
                when={d().format === "markdown"}
                fallback={<span class="break-all font-mono theme-muted">{d().content}</span>}
              >
                <div class="md-prose">{renderMd(d().content ?? "")}</div>
              </Show>
            </div>
          )}
        </Show>
      </details>
    </Panel>
  );
}
