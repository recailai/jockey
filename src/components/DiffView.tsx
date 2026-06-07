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

function rowClass(kind: Row["kind"]): string {
  if (kind === "add") return "diff-line diff-line-add";
  if (kind === "del") return "diff-line diff-line-del";
  if (kind === "hunk") return "diff-line diff-line-hunk";
  return "diff-line diff-line-ctx";
}

export default function DiffView(props: DiffViewProps) {
  const rows = createMemo(() => parseUnifiedDiff(props.diffText ?? ""));

  return (
    <div class="diff-view h-full overflow-auto theme-bg">
      <Show when={props.loading}>
        <div class="px-4 py-3 text-xs theme-muted">Loading diff…</div>
      </Show>
      <Show when={!props.loading && rows().length === 0}>
        <div class="px-4 py-3 text-xs theme-muted">{props.emptyLabel ?? "No changes"}</div>
      </Show>
      <Show when={!props.loading && rows().length > 0}>
        <div class="diff-table">
          <For each={rows()}>
            {(row) => {
              if (row.kind === "hunk") {
                return (
                  <div class="diff-line diff-line-hunk" title={row.text}>
                    <div class="diff-code diff-code-hunk">{row.text}</div>
                  </div>
                );
              }
              const sign = row.kind === "add" ? "+" : row.kind === "del" ? "−" : " ";
              return (
                <div class={rowClass(row.kind)}>
                  <div class="diff-gutter diff-gutter-old">{row.oldLn ?? ""}</div>
                  <div class="diff-gutter diff-gutter-new">{row.newLn ?? ""}</div>
                  <div class="diff-sign">{sign}</div>
                  <div class="diff-code">{row.text || " "}</div>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
