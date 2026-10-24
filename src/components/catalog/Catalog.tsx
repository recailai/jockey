import { For } from "solid-js";
import type { JSX } from "solid-js";
import { MessageSquare, Plus, RefreshCw } from "lucide-solid";
import {
  Alert,
  AsyncContent,
  Badge,
  Button,
  Checkbox,
  EmptyState,
  FormField,
  IconButton,
  Input,
  Pill,
  RowButton,
  StateDot,
  StatusBadge,
  StatusDot,
  Switch,
  ThemeToggle,
  ToolbarButton,
} from "../ui";
import { SESSION_STATUS_LOOK } from "../ui/status-badge";
import type { SessionStatus } from "../../lib/sessionStatus";
import { useTheme } from "../../lib/useTheme";
import { ToolCallGroup } from "../ToolCallGroup";
import type { AppToolCall } from "../types";
import "./catalog.css";

const TONES = ["neutral", "info", "success", "warning", "danger"] as const;
const STATUSES = Object.keys(SESSION_STATUS_LOOK) as SessionStatus[];

const tool = (id: string, title: string, kind: string, status: string): AppToolCall => ({ toolCallId: id, title, kind, status });
const TOOL_SAMPLES: { label: string; tools: AppToolCall[]; streaming?: boolean }[] = [
  { label: "Finished group", tools: [tool("a", "Read src/lib/theme.ts", "read", "completed"), tool("b", "pnpm build", "execute", "completed"), tool("c", "Edit src/index.css", "edit", "completed")] },
  { label: "Single call", tools: [tool("d", "cargo test --manifest-path src-tauri/Cargo.toml", "execute", "completed")] },
  { label: "Running", tools: [tool("e", "Read src/App.tsx", "read", "completed"), tool("f", "pnpm lint:ui", "execute", "running")], streaming: true },
  { label: "With a failure", tools: [tool("g", "git push", "execute", "failed"), tool("h", "git status", "execute", "completed")] },
  {
    label: "Sub-agent",
    streaming: true,
    tools: [
      {
        ...tool("subagent:c1", "explorer (Explore)", "agent", "in_progress"),
        toolName: "Agent",
        rawInput: { description: "Find the plan renderer", prompt: "Find where the plan panel renders and list its props." },
        outputLog: "The plan panel lives in `MessageWindow.tsx`.",
      },
      { ...tool("i", "Grep currentPlan", "search", "completed"), parentId: "subagent:c1" },
      { ...tool("j", "Read src/components/MessageWindow.tsx", "read", "in_progress"), parentId: "subagent:c1" },
    ],
  },
];

function Section(props: { title: string; note?: string; children: JSX.Element }) {
  return (
    <section class="catalog-section">
      <h2>{props.title}</h2>
      {props.note && <p class="catalog-note">{props.note}</p>}
      <div class="catalog-body">{props.children}</div>
    </section>
  );
}

/** A session list as both shells render it; the frame width drives the layout via container queries. */
function SessionListSample() {
  const rows: { title: string; meta: string; status: SessionStatus }[] = [
    { title: "Fix gateway SSE reconnect", meta: "jockey · @Developer · 3m", status: "needs-approval" },
    { title: "Run the test suite", meta: "jockey · @Developer · 12m", status: "working" },
    { title: "Pick a deploy target", meta: "notes · @Planner · 30m", status: "needs-answer" },
    { title: "Migrate settings", meta: "notes · @Developer · 1h", status: "external" },
    { title: "Refactor importer", meta: "jockey · @Reviewer · 2h", status: "error" },
    { title: "Draft release notes", meta: "notes · @Writer · 1d", status: "idle" },
  ];
  return (
    <div class="catalog-list">
      <For each={rows}>
        {(row) => (
          <RowButton class="catalog-list-row">
            <StatusDot status={row.status} />
            <span class="catalog-list-text">
              <span class="catalog-list-title">{row.title}</span>
              <span class="catalog-list-meta">{row.meta}</span>
            </span>
            <StatusBadge status={row.status} quiet class="catalog-list-badge" />
          </RowButton>
        )}
      </For>
    </div>
  );
}

/**
 * Dev-only living spec for `components/ui`. Open the desktop dev server at `/#catalog`.
 * Anything a view needs that is not shown here should become a primitive first.
 */
export default function Catalog() {
  const theme = useTheme();
  return (
    <main class="catalog">
      <header class="catalog-header">
        <div>
          <h1>Jockey UI catalog</h1>
          <p class="catalog-note">Primitives from <code>src/components/ui</code>, in the current theme. Rules: docs/architecture/ui-design-and-components.md</p>
        </div>
        <ThemeToggle preference={theme.preference} onCycle={theme.cycle} shortcut="⌘⇧L" />
      </header>

      <Section title="Session status" note="One vocabulary everywhere: lib/sessionStatus.ts → StatusBadge / StatusDot.">
        <div class="catalog-row">
          <For each={STATUSES}>{(s) => <StatusBadge status={s} />}</For>
        </div>
        <div class="catalog-row">
          <For each={STATUSES}>{(s) => <span class="catalog-chip"><StatusDot status={s} /> {s}</span>}</For>
        </div>
      </Section>

      <Section title="Adaptive layout" note="Same component in a phone-width and a panel-width frame. Layout follows the container, not the viewport.">
        <div class="catalog-frames">
          <div class="catalog-frame catalog-frame-phone"><SessionListSample /></div>
          <div class="catalog-frame catalog-frame-panel"><SessionListSample /></div>
        </div>
      </Section>

      <Section title="Agent tool groups" note="Collapsed process detail between replies. Clean runs show a quiet check; only exceptions get a badge.">
        <For each={TOOL_SAMPLES}>
          {(sample) => (
            <div>
              <p class="catalog-note">{sample.label}</p>
              <ToolCallGroup tools={sample.tools} streaming={!!sample.streaming} />
            </div>
          )}
        </For>
      </Section>

      <Section title="Buttons">
        <div class="catalog-row">
          <Button variant="primary">Primary</Button>
          <Button>Default</Button>
          <Button variant="secondary">Secondary</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="danger">Danger</Button>
          <Button disabled>Disabled</Button>
        </div>
        <div class="catalog-row">
          <Button size="sm">Small</Button>
          <Button size="md">Medium</Button>
          <Button size="lg">Large</Button>
          <IconButton title="Add" aria-label="Add"><Plus size={16} /></IconButton>
          <IconButton title="Refresh" aria-label="Refresh" variant="outline"><RefreshCw size={16} /></IconButton>
          <ToolbarButton title="Message" aria-label="Message"><MessageSquare size={15} /></ToolbarButton>
          <ToolbarButton title="Active" aria-label="Active" active><MessageSquare size={15} /></ToolbarButton>
        </div>
        <div class="catalog-row">
          <Pill>Static pill</Pill>
          <Pill onClick={() => {}}>Interactive</Pill>
          <Pill onClick={() => {}} active>Active</Pill>
        </div>
      </Section>

      <Section title="Badges">
        <div class="catalog-row">
          <For each={TONES}>{(t) => <Badge tone={t}>{t}</Badge>}</For>
        </div>
        <div class="catalog-row">
          <For each={TONES}>{(t) => <Badge tone={t} variant="outline">{t}</Badge>}</For>
        </div>
        <div class="catalog-row">
          <StateDot state="idle" /> <StateDot state="ongoing" /> <StateDot state="done" /> <StateDot state="warning" /> <StateDot state="error" />
        </div>
      </Section>

      <Section title="Alerts">
        <For each={TONES}>
          {(t) => <Alert tone={t} title={`${t} alert`} action={{ label: "Action", onClick: () => {} }}>Supporting text explains what happened and what to do next.</Alert>}
        </For>
      </Section>

      <Section title="Forms">
        <div class="catalog-grid">
          <FormField label="Name" description="Shown in the session list" required>
            <Input placeholder="Developer" />
          </FormField>
          <FormField label="With error" error="Name is already taken">
            <Input value="Reviewer" />
          </FormField>
          <Checkbox label="Auto-approve reads" description="Tool calls that only read files" />
          <Switch label="Follow the shown session" checked />
        </div>
      </Section>

      <Section title="Empty, loading and error states">
        <div class="catalog-grid">
          <AsyncContent state="loading" />
          <AsyncContent state="empty" emptyTitle="No active sessions" emptyDescription="Open a session to give an agent work." />
          <AsyncContent state="error" error="Gateway unreachable" onRetry={() => {}} />
          <EmptyState title="Nothing here yet" description="EmptyState can be used directly too." />
        </div>
      </Section>
    </main>
  );
}
