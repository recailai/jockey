import type { AppToolCall } from "../components/types";

/** Strips cwd prefix from an absolute path so the UI shows concise relative paths. */
export function stripCwdPrefix(filePath: string, cwd?: string | null): string {
  if (!filePath) return "";
  const normalized = filePath.replace(/\\/g, "/");
  if (!cwd) return normalized;
  const normalizedCwd = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  if (normalized.startsWith(normalizedCwd + "/")) {
    return normalized.slice(normalizedCwd.length + 1);
  }
  if (normalized === normalizedCwd) {
    return ".";
  }
  return normalized;
}

function parseJsonSafe(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      return null;
    }
  }
  return null;
}

export function humanizeToolName(name: string): string {
  const trimmed = (name || "").trim();
  if (!trimmed) return "Tool";
  // If it's already camelCase or has underscores/dashes, clean it up
  const words = trimmed
    .replace(/[._-]+/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .split(" ")
    .filter(Boolean);
  if (words.length === 0) return trimmed;
  return words
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase())
    .join(" ");
}

export interface ToolDisplayInfo {
  category: "shell" | "read" | "edit" | "write" | "search" | "fetch" | "task" | "tool";
  categoryBadge: string;
  actionLabel: string;
  targetSummary: string;
  secondaryInfo?: string;
}

export function formatToolDisplay(tc: AppToolCall, cwd?: string | null): ToolDisplayInfo {
  const name = (tc.title || tc.toolCallId || "").trim();
  const lowerName = name.toLowerCase();
  const rawInput = parseJsonSafe(tc.rawInput) || parseJsonSafe(tc.rawInputJson) || {};
  const firstLocation = tc.locations?.[0]?.path;

  // 1. Shell / Terminal Commands
  if (
    lowerName === "bash" ||
    lowerName === "terminal" ||
    lowerName === "run_command" ||
    lowerName === "execute_command" ||
    lowerName === "shell" ||
    tc.kind === "shell"
  ) {
    const cmd =
      (rawInput.command as string) ||
      (rawInput.cmd as string) ||
      (rawInput.CommandLine as string) ||
      "";
    return {
      category: "shell",
      categoryBadge: "Shell",
      actionLabel: "Run",
      targetSummary: cmd || "command",
    };
  }

  // 2. File Reading
  if (
    lowerName === "read_file" ||
    lowerName === "view_file" ||
    lowerName === "cat" ||
    lowerName.includes("read") ||
    tc.kind === "read"
  ) {
    const rawPath =
      (rawInput.filePath as string) ||
      (rawInput.path as string) ||
      (rawInput.AbsolutePath as string) ||
      (rawInput.targetFile as string) ||
      firstLocation ||
      "";
    const lineStart = rawInput.StartLine ?? rawInput.startLine ?? tc.locations?.[0]?.line;
    const lineEnd = rawInput.EndLine ?? rawInput.endLine;
    const lineSuffix = lineStart ? `:${lineStart}${lineEnd ? `-${lineEnd}` : ""}` : "";
    return {
      category: "read",
      categoryBadge: "Read",
      actionLabel: "Read",
      targetSummary: (stripCwdPrefix(rawPath, cwd) || "file") + lineSuffix,
    };
  }

  // 3. File Editing
  if (
    lowerName === "str_replace_editor" ||
    lowerName === "edit_file" ||
    lowerName === "replace_file_content" ||
    lowerName === "multi_replace_file_content" ||
    lowerName === "patch" ||
    lowerName.includes("edit") ||
    tc.kind === "edit"
  ) {
    const rawPath =
      (rawInput.filePath as string) ||
      (rawInput.path as string) ||
      (rawInput.TargetFile as string) ||
      (rawInput.targetFile as string) ||
      firstLocation ||
      "";
    return {
      category: "edit",
      categoryBadge: "Edit",
      actionLabel: "Edit",
      targetSummary: stripCwdPrefix(rawPath, cwd) || "file",
    };
  }

  // 4. File Writing / Creation
  if (
    lowerName === "write_to_file" ||
    lowerName === "create_file" ||
    lowerName === "write" ||
    lowerName.includes("write") ||
    tc.kind === "write"
  ) {
    const rawPath =
      (rawInput.filePath as string) ||
      (rawInput.path as string) ||
      (rawInput.TargetFile as string) ||
      (rawInput.targetFile as string) ||
      firstLocation ||
      "";
    return {
      category: "write",
      categoryBadge: "Write",
      actionLabel: "Create",
      targetSummary: stripCwdPrefix(rawPath, cwd) || "file",
    };
  }

  // 5. Search / Grep / Glob
  if (
    lowerName === "grep_search" ||
    lowerName === "file_search" ||
    lowerName === "search" ||
    lowerName === "find_by_name" ||
    lowerName.includes("grep") ||
    lowerName.includes("search")
  ) {
    const query =
      (rawInput.Query as string) ||
      (rawInput.query as string) ||
      (rawInput.pattern as string) ||
      "";
    const path =
      (rawInput.SearchPath as string) ||
      (rawInput.path as string) ||
      (rawInput.dir as string) ||
      "";
    const relativePath = path ? stripCwdPrefix(path, cwd) : "";
    return {
      category: "search",
      categoryBadge: "Search",
      actionLabel: "Search",
      targetSummary: query ? `"${query}"${relativePath ? ` in ${relativePath}` : ""}` : relativePath || "codebase",
    };
  }

  // 6. Network / Web Fetch
  if (
    lowerName === "read_url_content" ||
    lowerName === "web_search" ||
    lowerName === "search_web" ||
    lowerName === "fetch" ||
    lowerName === "browser_subagent"
  ) {
    const urlOrQuery =
      (rawInput.Url as string) ||
      (rawInput.url as string) ||
      (rawInput.query as string) ||
      (rawInput.TaskName as string) ||
      "";
    return {
      category: "fetch",
      categoryBadge: "Web",
      actionLabel: "Fetch",
      targetSummary: urlOrQuery || "web request",
    };
  }

  // 7. Subagents / Background Tasks
  if (lowerName.includes("subagent") || lowerName.includes("task") || lowerName === "manage_task") {
    const taskName =
      (rawInput.TaskName as string) ||
      (rawInput.taskName as string) ||
      (rawInput.description as string) ||
      "";
    return {
      category: "task",
      categoryBadge: "Agent",
      actionLabel: "Sub-agent",
      targetSummary: taskName || humanizeToolName(name),
    };
  }

  // 8. Fallback
  return {
    category: "tool",
    categoryBadge: "Tool",
    actionLabel: humanizeToolName(name),
    targetSummary: firstLocation ? stripCwdPrefix(firstLocation, cwd) : "",
  };
}
