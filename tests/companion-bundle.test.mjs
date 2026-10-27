import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";

test("production companion includes layout utilities from shared chat components", () => {
  const directory = new URL("../dist-companion/assets/", import.meta.url);
  const css = readdirSync(directory).filter((name) => name.endsWith(".css")).map((name) => readFileSync(new URL(name, directory), "utf8")).join("\n");
  for (const selector of [".items-center{", ".justify-end{", ".w-full{", ".flex-col{", ".min-h-0{", ".conversation-canvas", ".conversation-stream{"]) {
    assert.ok(css.includes(selector), `Shared chat layout missing from production: ${selector}`);
  }
});
