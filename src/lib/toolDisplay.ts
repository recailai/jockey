import type { AppToolCall } from "../components/types";
import { resolveToolVariant } from "./toolVariantRegistry";

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
  /** Command was dispatched non-blocking (Claude's `run_in_background`); its output
   *  arrives via a separate poll rather than this call's own result. */
  background?: boolean;
}

export function formatToolDisplay(tc: AppToolCall, cwd?: string | null): ToolDisplayInfo {
  const name = (tc.toolName || tc.title || tc.toolCallId || "").trim();
  const variant = resolveToolVariant(name, tc.kind);
  const rawInput = parseJsonSafe(tc.rawInput) || parseJsonSafe(tc.rawInputJson) || {};
  const firstLocation = tc.locations?.[0]?.path;

  // 1. Shell / Terminal Commands
  if (variant === "shell") {
    const commandValue =
      (rawInput.command as string) ||
      (rawInput.cmd as string) ||
      (rawInput.CommandLine as string) ||
      "";
    const cmd = Array.isArray(rawInput.command)
      ? rawInput.command.filter((part): part is string => typeof part === "string").join(" ")
      : commandValue;
    // Claude's Bash tool attaches a short human-readable `description` alongside
    // the literal command (e.g. "List files in src"); lead with that when present
    // and demote the exact command to a secondary line instead of dropping it.
    const description = (rawInput.description as string) || "";
    return {
      category: "shell",
      categoryBadge: "Shell",
      actionLabel: "Run",
      targetSummary: description || cmd || "command",
      secondaryInfo: description && cmd ? cmd : undefined,
      background: rawInput.run_in_background === true,
    };
  }

  // 2. File Reading
  if (variant === "read") {
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
  if (variant === "edit") {
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
  if (variant === "write") {
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
  if (variant === "search") {
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
  if (variant === "fetch") {
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
  if (variant === "task") {
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
