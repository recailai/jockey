import type { AppPermission, AppSegment, AppToolCall } from "../components/types";
import { isTransparentProviderEvent } from "./providerEventPolicy";

export type ProviderEventItem = { type: string; payload: unknown };

export type ToolCallGroupItem =
  | { kind: "tool"; tool: AppToolCall }
  | { kind: "events"; events: ProviderEventItem[] }
  | { kind: "thought"; text: string }
  | { kind: "executionText"; text: string };

export type ToolGroupBlock =
  | { kind: "text"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "tools"; items: ToolCallGroupItem[] }
  | { kind: "events"; events: ProviderEventItem[] };

/** Folds a flat segment stream into display blocks: text, thoughts, and tool groups. */
export function collectToolGroups(segments: AppSegment[]): ToolGroupBlock[] {
  const result: ToolGroupBlock[] = [];
  // A sub-agent's calls keep arriving while the parent writes; they join the group that holds
  // the sub-agent's row instead of starting a group of their own.
  const groupOfTool = new Map<string, ToolCallGroupItem[]>();
  for (const seg of segments) {
    if (seg.kind === "text") {
      const last = result[result.length - 1];
      if (last?.kind === "text") last.text += seg.text;
      else result.push({ kind: "text", text: seg.text });
    } else if (seg.kind === "tool") {
      const parentGroup = seg.tc.parentId ? groupOfTool.get(seg.tc.parentId) : undefined;
      if (parentGroup) {
        parentGroup.push({ kind: "tool", tool: seg.tc });
        groupOfTool.set(seg.tc.toolCallId, parentGroup);
        continue;
      }
      const last = result[result.length - 1];
      if (last?.kind === "tools") {
        last.items.push({ kind: "tool", tool: seg.tc });
      } else if (last?.kind === "events" || last?.kind === "thought") {
        const items: ToolCallGroupItem[] = [];
        while (result.length > 0) {
          const tail = result[result.length - 1];
          if (tail.kind === "events") {
            items.unshift({ kind: "events", events: tail.events });
            result.pop();
          } else if (tail.kind === "thought") {
            items.unshift({ kind: "thought", text: tail.text });
            result.pop();
          } else {
            break;
          }
        }
        items.push({ kind: "tool", tool: seg.tc });
        result.push({ kind: "tools", items });
      } else {
        result.push({ kind: "tools", items: [{ kind: "tool", tool: seg.tc }] });
      }
      const placed = result[result.length - 1];
      if (placed.kind === "tools") groupOfTool.set(seg.tc.toolCallId, placed.items);
    } else if (seg.kind === "thought") {
      const last = result[result.length - 1];
      if (last?.kind === "tools") {
        const tail = last.items[last.items.length - 1];
        if (tail?.kind === "thought") tail.text += seg.text;
        else last.items.push({ kind: "thought", text: seg.text });
      } else if (last?.kind === "thought") {
        last.text += seg.text;
      } else {
        result.push({ kind: "thought", text: seg.text });
      }
    } else {
      if (isTransparentProviderEvent({ type: seg.type, payload: seg.payload })) continue;
      const last = result[result.length - 1];
      const event = { type: seg.type, payload: seg.payload };
      if (last?.kind === "tools") {
        const tail = last.items[last.items.length - 1];
        if (tail?.kind === "events") tail.events.push(event);
        else last.items.push({ kind: "events", events: [event] });
      } else if (last?.kind === "events") {
        last.events.push(event);
      } else {
        result.push({ kind: "events", events: [event] });
      }
    }
  }
  return result;
}

export type ToolTreeIndex = {
  childMap: Map<string, AppToolCall[]>;
  nestedIds: Set<string>;
  /** The permission's owning row plus every ancestor sub-agent row; these render open. */
  openIds: Set<string>;
  ownerId: string | null;
};

/** The row a permission belongs to: the id the backend named when it is in this group,
 *  otherwise the latest waiting tool. Sub-agent rows also report `pending` (requires action)
 *  and must never be taken for the tool that is asking. */
export function permissionOwnerId(tools: AppToolCall[], permission: AppPermission | null | undefined): string | null {
  if (!permission) return null;
  const named = permission.toolCallId;
  if (named && tools.some((t) => t.toolCallId === named)) return named;
  for (let i = tools.length - 1; i >= 0; i--) {
    if (tools[i].status === "pending" && tools[i].kind !== "agent") return tools[i].toolCallId;
  }
  return null;
}

/** A call whose parent is in this group renders inside it; one whose parent is elsewhere
 *  (another group, or a provider that never sent it) stays at the top level. */
export function buildToolTree(tools: AppToolCall[], permission: AppPermission | null | undefined): ToolTreeIndex {
  const byId = new Map(tools.map((t) => [t.toolCallId, t] as const));
  const childMap = new Map<string, AppToolCall[]>();
  const nestedIds = new Set<string>();
  for (const t of tools) {
    if (!t.parentId || t.parentId === t.toolCallId || !byId.has(t.parentId)) continue;
    nestedIds.add(t.toolCallId);
    const list = childMap.get(t.parentId);
    if (list) list.push(t);
    else childMap.set(t.parentId, [t]);
  }
  const ownerId = permissionOwnerId(tools, permission);
  const openIds = new Set<string>();
  let cursor = ownerId;
  while (cursor && !openIds.has(cursor)) {
    openIds.add(cursor);
    const parent = byId.get(cursor)?.parentId;
    cursor = parent && byId.has(parent) ? parent : null;
  }
  return { childMap, nestedIds, openIds, ownerId };
}
