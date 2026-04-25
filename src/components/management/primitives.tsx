import { For, Show, createSignal, onCleanup } from "solid-js";
import { Portal } from "solid-js/web";
import { INTERACTIVE_MOTION } from "../types";

// ─────────────────────────────────────────────────────────────────────────────
// Domain types (backend shape — add to types.ts when wiring real invokes)
// ─────────────────────────────────────────────────────────────────────────────

export type StoredSession = {
  id: string;
  title: string;
  activeRole: string;
  runtimeKind: string | null;
  cwd: string | null;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
};

export type Workflow = {
  id: string;
  name: string;
  description: string;
  steps: WorkflowStep[];
  createdAt: number;
  updatedAt: number;
  status?: "idle" | "running" | "done" | "error";
};

export type WorkflowStep = {
  roleName: string;
  prompt: string;
  order: number;
};

export type McpServerStdio = {
  name: string;
  command: string;
  args: string[];
  env: Array<{ name: string; value: string }>;
};

export type McpServerHttp = {
  type: "http";
  name: string;
  url: string;
  headers: Array<{ name: string; value: string }>;
};

export type McpServerSse = {
  type: "sse";
  name: string;
  url: string;
  headers: Array<{ name: string; value: string }>;
};

export type AcpMcpServer = McpServerStdio | McpServerHttp | McpServerSse;

export function mcpTransport(s: AcpMcpServer): "stdio" | "http" | "sse" {
  if ("type" in s) return s.type;
  return "stdio";
}

export function mcpDisplayUri(s: AcpMcpServer): string {
  if ("command" in s) return `${s.command} ${s.args.join(" ")}`;
  return s.url;
}

export function parseCommandArgs(raw: string): string[] | null {
  const out: string[] = [];
  let buf = "";
  let quote: "'" | '"' | null = null;
  let escape = false;

  for (const ch of raw) {
    if (escape) {
      buf += ch;
      escape = false;
      continue;
    }
    if (ch === "\\") {
      escape = true;
      continue;
    }
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        buf += ch;
      }
      continue;
    }
    if (ch === "'" || ch === "\"") {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (buf) {
        out.push(buf);
        buf = "";
      }
      continue;
    }
    buf += ch;
  }

  if (escape || quote) return null;
  if (buf) out.push(buf);
  return out;
}

export type ContextEntry = { scope: string; key: string; value: string; updatedAt: number };

// ─────────────────────────────────────────────────────────────────────────────
// Primitive helpers
// ─────────────────────────────────────────────────────────────────────────────

export const fmtDate = (ts: number) =>
  new Date(ts).toLocaleDateString("en-US", {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false,
  });

export const fmtRelative = (ts: number) => {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared micro-components
// ─────────────────────────────────────────────────────────────────────────────

export function Badge(props: { label: string; color?: string; class?: string }) {
  return (
    <span class={`inline-flex items-center rounded-sm px-1.5 py-px font-mono text-[9px] font-semibold tracking-wide uppercase ${props.color ?? "theme-surface-muted theme-muted"} ${props.class ?? ""}`}>
      {props.label}
    </span>
  );
}

export function EmptyState(props: { icon: string; title: string; sub?: string }) {
  return (
    <div class="flex flex-col items-center justify-center gap-2 py-16 text-center">
      <span class="text-3xl opacity-20">{props.icon}</span>
      <p class="text-xs font-medium theme-muted">{props.title}</p>
      {props.sub && <p class="max-w-[200px] text-[10px] theme-muted opacity-60">{props.sub}</p>}
    </div>
  );
}

export function PanelSection(props: { title: string; action?: { label: string; onClick: () => void }; children: unknown }) {
  return (
    <div>
      <div class="mb-3 flex items-center justify-between pb-2">
        <span class="font-mono text-[9px] font-semibold uppercase tracking-[0.12em] theme-muted">{props.title}</span>
        <Show when={props.action}>
          {(action) => (
            <button
              onClick={action().onClick}
              class={`min-h-6 rounded border theme-border px-2 py-0.5 font-mono text-[9px] theme-muted hover:border-[var(--ui-border-strong)] hover:theme-text ${INTERACTIVE_MOTION}`}
            >
              {action().label}
            </button>
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
  const base = () => `w-full rounded-md border ${props.error ? "border-rose-600" : "theme-border"} theme-surface px-2 py-1 text-xs theme-text placeholder:text-[var(--ui-muted)] focus:border-[var(--ui-border-strong)] focus:outline-none ${props.monospace ? "font-mono" : ""} ${props.class ?? ""}`;
  if (props.multiline) {
    return (
      <textarea
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        placeholder={props.placeholder}
        rows={props.rows ?? 3}
        class={`${base()} resize-none`}
      />
    );
  }
  return (
    <input
      value={props.value}
      onInput={(e) => props.onInput(e.currentTarget.value)}
      placeholder={props.placeholder}
      class={`h-7 ${base()}`}
    />
  );
}

export function InlineSelect(props: {
  value: string;
  options: Array<{ value: string; label: string }>;
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
        class={`flex h-7 w-full items-center justify-between gap-2 rounded-md border theme-border theme-surface px-2 text-xs text-left ${INTERACTIVE_MOTION} ${open() ? "border-[var(--ui-border-strong)]" : "hover:border-[var(--ui-border-strong)]"}`}
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
            style={{ position: "fixed", top: `${pos().top}px`, left: `${pos().left}px`, width: `${pos().width}px` }}
            class="z-[9999] max-h-44 overflow-y-auto rounded-md shadow-xl shadow-black/60 theme-dropdown"
          >
            <For each={props.options}>
              {(opt) => (
                <button
                  data-isel
                  type="button"
                  onClick={() => { props.onChange(opt.value); close(); }}
                  class={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs ${INTERACTIVE_MOTION} ${opt.value === props.value ? "theme-dropdown-item-active" : "theme-dropdown-item"}`}
                >
                  <span class={`h-1 w-1 shrink-0 rounded-full ${opt.value === props.value ? "bg-emerald-400" : "bg-transparent"}`} />
                  {opt.label}
                </button>
              )}
            </For>
          </div>
        </Portal>
      </Show>
    </div>
  );
}

export function ActionButton(props: {
  onClick: () => void; label: string; variant?: "primary" | "danger" | "ghost"; class?: string; disabled?: boolean;
}) {
  const cls = () => ({
    primary: "bg-[var(--ui-text)] text-[var(--ui-bg)] hover:opacity-90 border-transparent",
    danger: "border-rose-800/60 text-rose-400 hover:bg-rose-500/10 hover:border-rose-600",
    ghost: "border-[var(--ui-border)] text-[var(--ui-muted)] hover:border-[var(--ui-border-strong)] hover:text-[var(--ui-text)]",
  }[props.variant ?? "ghost"]);
  return (
    <button
      onClick={props.onClick}
      disabled={props.disabled}
      class={`min-h-7 rounded-md border px-3 text-xs font-medium transition-all duration-150 active:scale-[0.98] disabled:opacity-40 disabled:pointer-events-none ${cls()} ${props.class ?? ""}`}
    >
      {props.label}
    </button>
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
    id: "workflows", label: "Workflows",
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
