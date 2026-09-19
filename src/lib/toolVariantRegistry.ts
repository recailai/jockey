import type { AppToolCall } from "../components/types";
import { createKeyedRegistry } from "./keyedRegistry";

export type ToolVariantCategory = "shell" | "read" | "edit" | "write" | "search" | "fetch" | "task" | "tool";

export type ToolVariant = {
  aliases: readonly string[];
  category: Exclude<ToolVariantCategory, "tool">;
  priority?: number;
};

const variants: ToolVariant[] = [
  { category: "shell", aliases: ["bash", "terminal", "run_command", "execute_command", "shell", "command", "commandexecution", "execute"] },
  { category: "read", aliases: ["read_file", "view_file", "cat"] },
  { category: "edit", aliases: ["str_replace_editor", "edit_file", "replace_file_content", "multi_replace_file_content", "patch"] },
  { category: "write", aliases: ["write_to_file", "create_file", "write"] },
  { category: "search", aliases: ["grep_search", "file_search", "search", "find_by_name", "glob"] },
  { category: "fetch", aliases: ["read_url_content", "web_search", "search_web", "fetch", "browser_subagent"] },
  { category: "task", aliases: ["subagent", "manage_task", "background_task"] },
];

const aliases = createKeyedRegistry<ToolVariant>();
for (const variant of variants) {
  for (const alias of variant.aliases) {
    aliases.register({ key: alias, value: variant, priority: variant.priority });
  }
}

export function registerToolVariant(variant: ToolVariant): void {
  for (const alias of variant.aliases) {
    aliases.register({ key: alias, value: variant, priority: variant.priority });
  }
}

export function resolveToolVariant(name: string, kind?: string): ToolVariantCategory {
  const normalizedName = name.trim().toLowerCase();
  const exact = aliases.get(normalizedName);
  if (exact) return exact.category;
  const explicitKind = kind?.trim().toLowerCase().replace(/[-_]/g, "");
  if (explicitKind === "execute" || explicitKind === "commandexecution" || explicitKind === "shell") {
    return "shell";
  }
  if (explicitKind === "filechange" || explicitKind === "edit") return "edit";
  if (explicitKind === "websearch" || explicitKind === "fetch") return "fetch";
  if (explicitKind === "mcptoolcall" || explicitKind === "dynamictoolcall") return "tool";
  if (explicitKind === "shell" || explicitKind === "read" || explicitKind === "edit" || explicitKind === "write" || explicitKind === "search" || explicitKind === "fetch" || explicitKind === "task") {
    return explicitKind;
  }
  return "tool";
}

export function toolVariantKey(tc: Pick<AppToolCall, "title" | "toolName" | "toolCallId" | "kind">): string {
  return (tc.toolName || tc.title || tc.toolCallId || tc.kind || "tool").trim().toLowerCase();
}
