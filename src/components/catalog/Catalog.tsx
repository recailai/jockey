import { For, createSignal } from "solid-js";
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
  Panel,
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
import type { AppPermission, AppPlanEntry, AppSession, AppToolCall, AppUserInputRequest } from "../types";
import { PlanPanel } from "../PlanPanel";
import { PermissionBody } from "../PermissionModal";
import UserInputModal from "../UserInputModal";
import "./catalog.css";

const TONES = ["neutral", "info", "success", "warning", "danger"] as const;
const STATUSES = Object.keys(SESSION_STATUS_LOOK) as SessionStatus[];

const tool = (id: string, title: string, kind: string, status: string): AppToolCall => ({ toolCallId: id, title, kind, status });
const agentRow = (id: string, title: string, status: string, extra: Partial<AppToolCall> = {}): AppToolCall => ({
  ...tool(`subagent:${id}`, title, "agent", status),
  toolName: "Agent",
  rawInput: { description: "Find the plan renderer", prompt: "Find where the plan panel renders and list its props." },
  ...extra,
});
const LONG_OUTPUT = Array.from({ length: 40 }, (_, i) => `${i + 1}. Finding ${i + 1}: a long line of sub-agent output that must wrap inside the row instead of widening the page, e.g. src/components/very/long/path/to/File${i}.tsx`).join("\n");
const PERMISSION: AppPermission = {
  requestId: "catalog-permission",
  toolCallId: "p2",
  title: "Run pnpm install?",
  description: null,
  options: [{ optionId: "allow", title: "Allow", kind: "allow_once" }, { optionId: "reject", title: "Reject", kind: "reject_once" }],
};
const PLAN_APPROVAL: AppPermission = {
  requestId: "catalog-plan-approval",
  title: "Ready to code?",
  description: null,
  toolKind: "switch_mode",
  content: [{ type: "content", content: { type: "text", text: "## Plan\n\n1. Extract `PermissionBody` from the modal\n2. Render the plan above the options\n3. Hide the Remember switch for plan approvals\n\n" + "A long plan scrolls inside its box instead of pushing the options away.\n\n".repeat(8) } }],
  options: [
    { optionId: "clear-auto", title: "Yes, clear context and use auto mode", kind: "allow_always" },
    { optionId: "auto", title: "Yes, and use auto mode", kind: "allow_always" },
    { optionId: "manual", title: "Yes, manually approve edits", kind: "allow_once" },
    { optionId: "keep", title: "No, keep planning", kind: "reject_once" },
  ],
};
const USER_INPUT: AppUserInputRequest = {
  requestId: "catalog-user-input",
  title: "Please answer the following questions.",
  blocking: true,
  questions: [
    {
      id: "question_0", header: "Library", prompt: "Which date library should the form use?", allowOther: true, secret: false, multi: false,
      options: [
        { value: "date-fns (Recommended)", label: "date-fns (Recommended)", description: "Small, tree-shakeable helpers" },
        { value: "Temporal", label: "Temporal", description: "Native, needs a polyfill today" },
      ],
    },
    {
      id: "question_1", header: "Features", prompt: "Which features should it support?", allowOther: true, secret: false, multi: true,
      options: [
        { value: "Ranges", label: "Ranges" },
        { value: "Time zones", label: "Time zones", description: "Needs the IANA database" },
        { value: "Recurring", label: "Recurring" },
      ],
    },
    { id: "question_2", header: "Token", prompt: "API token for the preview server", allowOther: false, secret: true, multi: false, options: [] },
  ],
};
const PLAN: AppPlanEntry[] = [
  { content: "Read the plan renderer", status: "completed" },
  { content: "Extract it into a component", status: "in_progress", priority: "high" },
  { content: "Check both themes", status: "pending", priority: "medium" },
];
const TOOL_SAMPLES: { label: string; tools: AppToolCall[]; streaming?: boolean; permission?: AppPermission }[] = [
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
  {
    label: "Sub-agent, finished (no stop)",
    tools: [
      agentRow("c2", "explorer (Explore)", "completed", { outputLog: "The plan panel lives in `PlanPanel.tsx`." }),
      { ...tool("k", "Grep currentPlan", "search", "completed"), parentId: "subagent:c2" },
    ],
  },
  {
    label: "Nested sub-agents, depth 3",
    streaming: true,
    tools: [
      agentRow("d1", "lead (General)", "in_progress"),
      { ...agentRow("d2", "explorer (Explore)", "in_progress"), parentId: "subagent:d1" },
      { ...agentRow("d3", "reader (Explore)", "in_progress"), parentId: "subagent:d2" },
      { ...tool("l", "Read src/index.css", "read", "in_progress"), parentId: "subagent:d3" },
    ],
  },
  {
    label: "Sub-agent with long output",
    tools: [agentRow("e1", "reviewer (Review)", "completed", { outputLog: LONG_OUTPUT })],
  },
  {
    label: "Child awaiting approval",
    streaming: true,
    permission: PERMISSION,
    tools: [
      agentRow("f1", "builder (General)", "pending"),
      { ...tool("p1", "Read package.json", "read", "completed"), parentId: "subagent:f1" },
      { ...tool("p2", "pnpm install", "execute", "pending"), parentId: "subagent:f1" },
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
  const [userInputSession, setUserInputSession] = createSignal({ id: "catalog", pendingUserInput: [USER_INPUT] } as unknown as AppSession);
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
              <ToolCallGroup tools={sample.tools} streaming={!!sample.streaming} pendingPermission={sample.permission} onStopAgent={() => {}} />
            </div>
          )}
        </For>
      </Section>

      <Section title="Tool rows at phone width">
        <div class="catalog-frame catalog-frame-phone">
          <ToolCallGroup streaming tools={[{
            ...tool("narrow-shell", "Bash", "execute", "running"),
            toolName: "Bash",
            rawInput: {
              description: "Check session recovery and role bindings across every configured runtime",
              command: "cargo test --manifest-path src-tauri/Cargo.toml db::app_session_role::binding_tests",
            },
          }]} />
        </div>
      </Section>

      <Section title="Plan approval" note="A plan approval shows every option as the agent sent it, with no Remember switch.">
        <Panel class="permission-panel my-2 px-3 py-2">
          <PermissionBody permission={PLAN_APPROVAL} onChoose={() => {}} onDeny={() => {}} />
        </Panel>
      </Section>

      <Section title="Questions" note="Several questions become tabs with a final review step.">
        <UserInputModal inline activeSession={userInputSession} patchActiveSession={(patch) => setUserInputSession((s) => ({ ...s, ...patch }) as AppSession)} />
      </Section>

      <Section title="Plan">
        <PlanPanel plan={PLAN} />
        <PlanPanel plan={[{ content: "# Plan\n\n- one\n- two", format: "markdown" }]} />
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
