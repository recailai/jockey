import { For, Show, createSignal } from "solid-js";
import type { ParsedDiff, DiffHunk } from "../lib/diffParser";
import { INTERACTIVE_MOTION } from "./types";

type HunkStatus = "kept" | "rejected" | "pending";

function lineClass(kind: "context" | "add" | "remove") {
  if (kind === "add") return "bg-emerald-500/10 text-emerald-200";
  if (kind === "remove") return "bg-rose-500/10 text-rose-300 line-through opacity-70";
  return "theme-muted";
}

function linePrefix(kind: "context" | "add" | "remove") {
  if (kind === "add") return "+";
  if (kind === "remove") return "-";
  return " ";
}

type HunkProps = {
  hunk: DiffHunk;
  filePath: string;
  status: HunkStatus;
  onReject: () => void;
  onKeep: () => void;
};

function HunkBlock(props: HunkProps) {
  return (
    <div
      class="rounded-lg border overflow-hidden"
      classList={{
        "border-emerald-500/30 bg-emerald-500/5": props.status === "kept",
        "border-rose-500/30 bg-rose-500/5 opacity-60": props.status === "rejected",
        "border-white/10 theme-surface": props.status === "pending",
      }}
    >
      <div class="flex items-center justify-between px-3 py-1.5 border-b border-white/5">
        <span class="font-mono text-[9px] theme-muted">{props.hunk.header}</span>
        <div class="flex items-center gap-1.5">
          <Show when={props.status === "rejected"}>
            <span class="text-[9px] text-rose-400 font-semibold">rejected</span>
          </Show>
          <Show when={props.status === "kept"}>
            <span class="text-[9px] text-emerald-400 font-semibold">kept</span>
          </Show>
          <Show when={props.status !== "rejected"}>
            <button
              type="button"
              onClick={props.onReject}
              class={`rounded border border-rose-500/40 bg-rose-500/10 px-2 py-0.5 text-[9px] font-semibold text-rose-300 hover:bg-rose-500/20 ${INTERACTIVE_MOTION}`}
            >
              Reject
            </button>
          </Show>
          <Show when={props.status !== "kept"}>
            <button
              type="button"
              onClick={props.onKeep}
              class={`rounded border border-emerald-500/40 bg-emerald-500/10 px-2 py-0.5 text-[9px] font-semibold text-emerald-300 hover:bg-emerald-500/20 ${INTERACTIVE_MOTION}`}
            >
              Keep
            </button>
          </Show>
        </div>
      </div>
      <pre class="overflow-x-auto px-3 py-2 text-[10.5px] font-mono leading-[1.45]">
        <For each={props.hunk.lines}>{(line) => (
          <div class={`flex gap-2 ${lineClass(line.kind)}`}>
            <span class="w-3 shrink-0 select-none opacity-50">{linePrefix(line.kind)}</span>
            <span class="break-all whitespace-pre-wrap">{line.text}</span>
          </div>
        )}</For>
      </pre>
    </div>
  );
}

type Props = {
  diffs: ParsedDiff[];
  onRejectHunk?: (filePath: string, hunk: DiffHunk) => void;
};

export function DiffViewer(props: Props) {
  const makeKey = (fileIdx: number, hunkIdx: number) => `${fileIdx}:${hunkIdx}`;
  const [statuses, setStatuses] = createSignal<Record<string, HunkStatus>>({});

  const setStatus = (key: string, s: HunkStatus) =>
    setStatuses((prev) => ({ ...prev, [key]: s }));

  const getStatus = (key: string): HunkStatus => statuses()[key] ?? "pending";

  return (
    <div class="space-y-4">
      <For each={props.diffs}>{(diff, fi) => (
        <div class="space-y-2">
          <div class="flex items-center gap-2 px-1">
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 theme-muted"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
            <span class="font-mono text-[10.5px] theme-text font-semibold truncate">
              {diff.toFile.replace(/^[ab]\//, "")}
            </span>
            <span class="text-[9px] theme-muted">{diff.hunks.length} hunk{diff.hunks.length !== 1 ? "s" : ""}</span>
          </div>
          <For each={diff.hunks}>{(hunk, hi) => {
            const key = () => makeKey(fi(), hi());
            return (
              <HunkBlock
                hunk={hunk}
                filePath={diff.toFile.replace(/^[ab]\//, "")}
                status={getStatus(key())}
                onKeep={() => setStatus(key(), "kept")}
                onReject={() => {
                  setStatus(key(), "rejected");
                  props.onRejectHunk?.(diff.toFile.replace(/^[ab]\//, ""), hunk);
                }}
              />
            );
          }}</For>
        </div>
      )}</For>
    </div>
  );
}
