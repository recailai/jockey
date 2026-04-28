import { For, Show, createMemo } from "solid-js";

type DiffViewProps = {
  diffText: string;
  loading?: boolean;
  emptyLabel?: string;
};

type Row =
  | { kind: "hunk"; text: string }
  | { kind: "ctx" | "add" | "del"; oldLn: number | null; newLn: number | null; text: string };

const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

function parseUnifiedDiff(text: string): Row[] {
  if (!text) return [];
  const rows: Row[] = [];
  let oldLn = 0;
  let newLn = 0;
  let inHunk = false;

  for (const raw of text.split("\n")) {
    if (raw.startsWith("@@")) {
      const m = HUNK_RE.exec(raw);
      if (m) {
        oldLn = parseInt(m[1], 10);
        newLn = parseInt(m[2], 10);
        inHunk = true;
      }
      rows.push({ kind: "hunk", text: raw });
      continue;
    }
    if (!inHunk) continue;
    const first = raw[0] ?? " ";
    if (first === "+") {
      rows.push({ kind: "add", oldLn: null, newLn: newLn, text: raw.slice(1) });
      newLn += 1;
    } else if (first === "-") {
      rows.push({ kind: "del", oldLn: oldLn, newLn: null, text: raw.slice(1) });
      oldLn += 1;
    } else if (first === "\\") {
      continue;
    } else {
      rows.push({ kind: "ctx", oldLn: oldLn, newLn: newLn, text: raw.slice(first === " " ? 1 : 0) });
      oldLn += 1;
      newLn += 1;
    }
  }
  const last = rows[rows.length - 1];
  if (last && last.kind === "ctx" && last.text === "") {
    rows.pop();
  }
  return rows;
}

export default function DiffView(props: DiffViewProps) {
  const rows = createMemo(() => parseUnifiedDiff(props.diffText ?? ""));

  return (
    <div class="h-full overflow-auto font-mono text-[12px] leading-[1.6] theme-bg">
      <Show when={props.loading}>
        <div class="px-4 py-3 text-xs theme-muted">Loading diff…</div>
      </Show>
      <Show when={!props.loading && rows().length === 0}>
        <div class="px-4 py-3 text-xs theme-muted">{props.emptyLabel ?? "No changes"}</div>
      </Show>
      <Show when={!props.loading && rows().length > 0}>
        <div
          class="diff-grid"
          style={{
            display: "grid",
            "grid-template-columns": "auto auto auto minmax(max-content, 1fr)",
          }}
        >
          <For each={rows()}>
            {(row) => {
              if (row.kind === "hunk") {
                return (
                  <div
                    class="col-span-4 px-4 py-0.5 text-[11px] select-text"
                    style={{
                      "color": "var(--ui-accent)",
                      "background-color": "var(--ui-accent-soft)",
                      "border-top": "1px solid var(--ui-border)",
                      "border-bottom": "1px solid var(--ui-border)",
                    }}
                    title={row.text}
                  >
                    {row.text}
                  </div>
                );
              }
              const rowBg =
                row.kind === "add"
                  ? "bg-emerald-500/10"
                  : row.kind === "del"
                  ? "bg-rose-500/10"
                  : "";
              const sign = row.kind === "add" ? "+" : row.kind === "del" ? "−" : " ";
              const signColor =
                row.kind === "add"
                  ? "text-emerald-500"
                  : row.kind === "del"
                  ? "text-rose-500"
                  : "theme-muted";
              return (
                <>
                  {/* gutter: old line no */}
                  <div
                    class={`sticky left-0 z-[1] pl-3 pr-1.5 text-right tabular-nums select-none min-w-[3rem] theme-muted ${rowBg}`}
                    style={{ "background-color": rowBg ? undefined : "var(--ui-surface-muted)" }}
                  >
                    {row.oldLn ?? ""}
                  </div>
                  {/* gutter: new line no */}
                  <div
                    class={`pl-1.5 pr-2 text-right tabular-nums select-none min-w-[3rem] theme-muted ${rowBg}`}
                    style={{ "background-color": rowBg ? undefined : "var(--ui-surface-muted)" }}
                  >
                    {row.newLn ?? ""}
                  </div>
                  {/* sign */}
                  <div class={`pl-2 pr-1 select-none ${signColor} ${rowBg}`}>
                    {sign}
                  </div>
                  {/* content */}
                  <div class={`pr-4 whitespace-pre theme-text ${rowBg}`}>
                    {row.text || " "}
                  </div>
                </>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
