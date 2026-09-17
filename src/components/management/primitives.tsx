import { For, Show, createSignal, onCleanup } from "solid-js";
import { Portal } from "solid-js/web";
import { INTERACTIVE_MOTION } from "../types";
import { Badge as UiBadge, Button, EmptyState as UiEmptyState, Input, Textarea } from "../ui";

// Re-export domain types from canonical location (src/components/types.ts)
export type {
  StoredSession,
  Workflow,
  WorkflowStep,
  McpServerStdio,
  McpServerHttp,
  McpServerSse,
  AcpMcpServer,
  ContextEntry,
  TabId,
} from "../types";

// Re-export helpers from canonical locations
export { mcpTransport, mcpDisplayUri, parseCommandArgs } from "../../lib/mcpHelpers";
export { fmtDate, fmtRelative } from "../../lib/formatHelpers";

// ─────────────────────────────────────────────────────────────────────────────
// Shared micro-components
// ─────────────────────────────────────────────────────────────────────────────

/** @deprecated Use `Badge` from `src/components/ui` instead. */
export function Badge(props: { label: string; color?: string; class?: string }) {
  return (
    <span class={`ui-badge ${props.color ?? ""} ${props.class ?? ""}`}>
      {props.label}
    </span>
  );
}

/** @deprecated Use `EmptyState` from `src/components/ui` instead. */
export function EmptyState(props: { icon: string; title: string; sub?: string }) {
  return (
    <UiEmptyState icon={props.icon} title={props.title} description={props.sub} />
  );
}

export function PanelSection(props: { title: string; action?: { label: string; onClick: () => void }; children: unknown }) {
  return (
    <div>
      <div class="mb-3 flex items-center justify-between pb-2">
        <span class="font-mono text-[9px] font-semibold uppercase tracking-[0.12em] theme-muted">{props.title}</span>
        <Show when={props.action}>
          {(action) => (
            <Button
              variant="outline"
              size="sm"
              onClick={action().onClick}
              class="font-mono text-[10px]"
            >
              {action().label}
            </Button>
          )}
        </Show>
      </div>
      {props.children as any}
    </div>
  );
}

export function FieldRow(props: { label: string; children: unknown }) {
  return (
    <div class="flex items-start gap-3">
      <span class="w-20 shrink-0 font-mono text-[9px] uppercase tracking-wide theme-muted pt-[3px]">{props.label}</span>
      <div class="flex-1 min-w-0">{props.children as any}</div>
    </div>
  );
}

export function TextInput(props: {
  value: string; onInput: (v: string) => void; placeholder?: string;
  multiline?: boolean; rows?: number; class?: string; monospace?: boolean; error?: boolean;
}) {
  const base = () => `text-xs placeholder:text-[var(--ui-muted)] ${props.error ? "border-[var(--ui-state-danger-text)]" : ""} ${props.monospace ? "font-mono" : ""} ${props.class ?? ""}`;
  if (props.multiline) {
    return (
      <Textarea
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        placeholder={props.placeholder}
        rows={props.rows ?? 3}
        class={`${base()} resize-none`}
      />
    );
  }
  return (
    <Input
      value={props.value}
      onInput={(e) => props.onInput(e.currentTarget.value)}
      placeholder={props.placeholder}
      class={base()}
    />
  );
}

export type InlineSelectOption = {
  value: string;
  label: string;
  group?: string;
  disabled?: boolean;
  hint?: string;
};

type GroupedEntry =
  | { header: string }
  | { opt: InlineSelectOption };

/** Group options by their `group` field; ungrouped options render without
 *  headers. Returns a flat, pre-ordered list for `<For>`. */
function groupedOptions(options: Array<InlineSelectOption>): Array<GroupedEntry> {
  const hasGroups = options.some((opt) => !!opt.group);
  if (!hasGroups) return options.map((opt) => ({ opt }));

  const groups = new Map<string, InlineSelectOption[]>();
  const ungrouped: InlineSelectOption[] = [];

  for (const opt of options) {
    if (opt.group) {
      const list = groups.get(opt.group) ?? [];
      list.push(opt);
      groups.set(opt.group, list);
    } else {
      ungrouped.push(opt);
    }
  }

  const out: Array<GroupedEntry> = [];
  for (const [header, list] of groups.entries()) {
    out.push({ header });
    for (const opt of list) {
      out.push({ opt });
    }
  }
  for (const opt of ungrouped) {
    out.push({ opt });
  }
  return out;
}

export function InlineSelect(props: {
  value: string;
  options: Array<InlineSelectOption>;
  onChange: (v: string) => void;
  class?: string;
}) {
  const [open, setOpen] = createSignal(false);
  const [pos, setPos] = createSignal({ top: 0, left: 0, width: 0 });
  let triggerRef: HTMLButtonElement | undefined;
  const selected = () => props.options.find((o) => o.value === props.value);

  const close = () => {
    setOpen(false);
    document.removeEventListener("click", handleOutside, true);
  };

  const handleOutside = (e: MouseEvent) => {
    if (!(e.target as Element).closest("[data-isel]")) close();
  };

  const toggle = () => {
    if (open()) { close(); return; }
    if (triggerRef) {
      const r = triggerRef.getBoundingClientRect();
      setPos({ top: r.bottom + 2, left: r.left, width: r.width });
    }
    setOpen(true);
    setTimeout(() => document.addEventListener("click", handleOutside, true), 0);
  };

  onCleanup(() => document.removeEventListener("click", handleOutside, true));

  return (
    <div data-isel class={`relative ${props.class ?? ""}`}>
      <button
        ref={triggerRef}
        type="button"
        onClick={toggle}
        title={selected()?.label ?? "Select..."}
        class={`jui-field flex h-[var(--ui-control-height-sm)] w-full items-center justify-between gap-2 px-2 text-xs text-left ${INTERACTIVE_MOTION} ${open() ? "border-[var(--ui-border-strong)]" : "hover:border-[var(--ui-border-strong)]"}`}
      >
        <span class={`min-w-0 flex-1 truncate ${selected() ? "theme-text" : "theme-muted"}`}>
          {selected()?.label ?? "Select…"}
        </span>
        <svg class={`h-3 w-3 shrink-0 theme-muted transition-transform ${open() ? "rotate-180" : ""}`} viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 4l4 4 4-4" /></svg>
      </button>
      <Show when={open()}>
        <Portal mount={document.body}>
          <div
            data-isel
            style={{ position: "fixed", top: `${pos().top}px`, left: `${pos().left}px`, width: `${Math.max(pos().width, 260)}px`, "max-width": "min(520px, calc(100vw - 24px))", "z-index": "var(--z-dropdown)" }}
            class="max-h-44 overflow-y-auto theme-dropdown"
          >
            <For each={groupedOptions(props.options)}>{(entry) =>
              "header" in entry ? (
                <div class="px-2 pt-1.5 pb-0.5 text-[9px] font-bold uppercase tracking-widest theme-muted">{entry.header}</div>
              ) : (
                <button
                  data-isel
                  type="button"
                  title={entry.opt.hint ?? entry.opt.label}
                  disabled={entry.opt.disabled}
                  onClick={() => { props.onChange(entry.opt.value); close(); }}
                  class={`completion-row items-start py-1.5 text-xs ${INTERACTIVE_MOTION} ${entry.opt.value === props.value ? "theme-dropdown-item-active" : "theme-dropdown-item"} ${entry.opt.disabled ? "cursor-not-allowed opacity-40" : ""}`}
                >
                  <span class={`mt-1.5 settings-runtime-dot shrink-0 ${entry.opt.value === props.value ? "is-online" : "opacity-0"}`} />
                  <span class="min-w-0 break-words">{entry.opt.label}</span>
                </button>
              )
            }</For>
          </div>
        </Portal>
      </Show>
    </div>
  );
}

export function ActionButton(props: {
  onClick: () => void; label: string; variant?: "primary" | "danger" | "ghost"; class?: string; disabled?: boolean;
}) {
  return (
    <Button
      onClick={props.onClick}
      disabled={props.disabled}
      variant={props.variant === "primary" ? "default" : props.variant === "danger" ? "destructive" : "outline"}
      size="sm"
      class={`text-xs ${props.class ?? ""}`}
    >
      {props.label}
    </Button>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Nav tab definitions
// ─────────────────────────────────────────────────────────────────────────────

export type TabId = "sessions" | "workflows" | "roles" | "mcp" | "skills" | "rules" | "agents";

export const TABS: Array<{ id: TabId; label: string; icon: () => any }> = [
  {
    id: "agents", label: "Agents",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>
      </svg>
    ),
  },
  {
    id: "sessions", label: "Sessions",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <rect x="3" y="3" width="18" height="18" rx="2" /><line x1="3" y1="9" x2="21" y2="9" /><line x1="9" y1="21" x2="9" y2="9" />
      </svg>
    ),
  },
  {
    id: "workflows", label: "Automations",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="18" cy="5" r="3" /><circle cx="6" cy="12" r="3" /><circle cx="18" cy="19" r="3" />
        <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" /><line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
      </svg>
    ),
  },
  {
    id: "roles", label: "Roles",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" />
      </svg>
    ),
  },
  {
    id: "mcp", label: "MCP",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <polygon points="12 2 2 7 12 12 22 7 12 2" /><polyline points="2 17 12 22 22 17" /><polyline points="2 12 12 17 22 12" />
      </svg>
    ),
  },
  {
    id: "skills", label: "Skills",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
      </svg>
    ),
  },
  {
    id: "rules", label: "Rules",
    icon: () => (
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" /><polyline points="10 9 9 9 8 9" />
      </svg>
    ),
  },

];
