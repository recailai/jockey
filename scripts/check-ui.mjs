#!/usr/bin/env node
// UI guardrails (docs/architecture/ui-design-and-components.md). Runs before every build.
//
//   node scripts/check-ui.mjs            check
//   node scripts/check-ui.mjs --update   shrink the raw-<button> baseline after a cleanup
//
// Hard rules fail at any count. Ratcheted rules may only go down per file: the baseline records
// today's debt so new code cannot add to it, and --update locks in each cleanup.
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const SRC = join(ROOT, "src");
const BASELINE = join(ROOT, "scripts", "ui-baseline.json");
const PRIMITIVES = "src/components/ui/";

const HARD = [
  {
    name: "palette color class",
    hint: "use --ui-* tokens or a tone prop (Badge/Alert/StatusBadge)",
    re: /\b(?:text|bg|border|ring|fill|stroke|accent|from|to|via)-(?:red|rose|pink|fuchsia|purple|violet|indigo|blue|sky|cyan|teal|emerald|green|lime|yellow|amber|orange|slate|gray|zinc|neutral|stone)-\d{2,3}\b/g,
  },
  {
    name: "raw hex color in markup",
    hint: "colors live in index.css tokens",
    re: /(?:class|style)=[{"'`][^>\n]*#[0-9a-fA-F]{3,8}\b/g,
    skip: (file) => file.startsWith(PRIMITIVES) || file.endsWith("lib/theme.ts"),
  },
];

const RATCHETED = [
  {
    key: "offScaleRadius",
    name: "off-scale border-radius",
    hint: "use var(--ui-radius-*) (4/8/12/16/20/28), 999px for pills or 50% for circles",
    re: /border-radius:\s*(?!(?:0|4|8|12|16|20|28|999)px\b)[0-9.]+px/g,
    css: true,
  },
  {
    key: "rawButton",
    name: "raw <button>",
    hint: "use Button / IconButton / ToolbarButton / RowButton / Pill from components/ui",
    re: /<button\b/g,
    skip: (file) => file.startsWith(PRIMITIVES),
  },
];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(tsx|ts|css)$/.test(name)) out.push(path);
  }
  return out;
}

const files = walk(SRC).map((p) => ({ rel: relative(ROOT, p), text: readFileSync(p, "utf8") }));
const lineOf = (text, index) => text.slice(0, index).split("\n").length;

let failed = false;
const appliesTo = (rule, file) => (rule.css ? file.endsWith(".css") : !file.endsWith(".css"));

for (const rule of HARD) {
  for (const f of files) {
    if (!appliesTo(rule, f.rel) || rule.skip?.(f.rel)) continue;
    for (const m of f.text.matchAll(rule.re)) {
      failed = true;
      console.error(`${f.rel}:${lineOf(f.text, m.index)}  ${rule.name}: ${m[0].slice(0, 60)}  → ${rule.hint}`);
    }
  }
}

const baseline = (() => {
  try {
    return JSON.parse(readFileSync(BASELINE, "utf8"));
  } catch {
    return {};
  }
})();
const next = {};
for (const rule of RATCHETED) {
  next[rule.key] = {};
  for (const f of files) {
    if (!appliesTo(rule, f.rel) || rule.skip?.(f.rel)) continue;
    const count = [...f.text.matchAll(rule.re)].length;
    if (!count) continue;
    next[rule.key][f.rel] = count;
    const allowed = baseline[rule.key]?.[f.rel] ?? 0;
    if (count > allowed) {
      failed = true;
      console.error(`${f.rel}  ${rule.name}: ${count} (baseline ${allowed})  → ${rule.hint}`);
    }
  }
}

if (process.argv.includes("--update")) {
  for (const rule of RATCHETED) {
    for (const [file, count] of Object.entries(next[rule.key])) {
      const allowed = baseline[rule.key]?.[file];
      // Only ever lowers counts; raising one needs a deliberate edit of the JSON in review.
      if (allowed !== undefined && count > allowed) next[rule.key][file] = allowed;
    }
  }
  writeFileSync(BASELINE, `${JSON.stringify(next, null, 2)}\n`);
  console.log(`updated ${relative(ROOT, BASELINE)}`);
}

if (failed) {
  console.error("\nUI check failed. See docs/architecture/ui-design-and-components.md.");
  process.exit(1);
}
console.log("UI check passed.");
