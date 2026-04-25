export type DiffLine = {
  kind: "context" | "add" | "remove";
  text: string;
  oldLine?: number;
  newLine?: number;
};

export type DiffHunk = {
  header: string;
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: DiffLine[];
};

export type ParsedDiff = {
  fromFile: string;
  toFile: string;
  hunks: DiffHunk[];
};

export function parseDiff(raw: string): ParsedDiff[] {
  const result: ParsedDiff[] = [];
  const lines = raw.split("\n");
  let current: ParsedDiff | null = null;
  let currentHunk: DiffHunk | null = null;
  let oldLine = 0;
  let newLine = 0;

  for (const line of lines) {
    if (line.startsWith("--- ")) {
      if (current) result.push(current);
      current = { fromFile: line.slice(4).split("\t")[0].trim(), toFile: "", hunks: [] };
      currentHunk = null;
      continue;
    }
    if (line.startsWith("+++ ") && current) {
      current.toFile = line.slice(4).split("\t")[0].trim();
      continue;
    }
    if (line.startsWith("@@ ") && current) {
      const m = line.match(/@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)/);
      if (m) {
        if (currentHunk) current.hunks.push(currentHunk);
        oldLine = parseInt(m[1], 10);
        newLine = parseInt(m[3], 10);
        currentHunk = {
          header: line,
          oldStart: oldLine,
          oldCount: m[2] !== undefined ? parseInt(m[2], 10) : 1,
          newStart: newLine,
          newCount: m[4] !== undefined ? parseInt(m[4], 10) : 1,
          lines: [],
        };
      }
      continue;
    }
    if (!currentHunk) continue;
    if (line.startsWith("+")) {
      currentHunk.lines.push({ kind: "add", text: line.slice(1), newLine: newLine++ });
    } else if (line.startsWith("-")) {
      currentHunk.lines.push({ kind: "remove", text: line.slice(1), oldLine: oldLine++ });
    } else if (line.startsWith(" ") || line === "") {
      currentHunk.lines.push({ kind: "context", text: line.slice(1), oldLine: oldLine++, newLine: newLine++ });
    }
  }

  if (currentHunk && current) current.hunks.push(currentHunk);
  if (current) result.push(current);
  return result;
}

export function isDiffLike(text: string): boolean {
  return /^(---|\+\+\+|@@\s+-\d+)/m.test(text);
}

export function hunkToRejectPrompt(filePath: string, hunk: DiffHunk): string {
  const removedLines = hunk.lines.filter((l) => l.kind === "remove").map((l) => l.text);
  const addedLines = hunk.lines.filter((l) => l.kind === "add").map((l) => l.text);
  return [
    `Please revert the following change in \`${filePath}\` (hunk starting at line ${hunk.newStart}):`,
    ``,
    `The agent added:`,
    ...addedLines.map((l) => `  + ${l}`),
    ``,
    `Please restore the original:`,
    ...removedLines.map((l) => `  - ${l}`),
  ].join("\n");
}
