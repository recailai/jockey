# Jockey UI Design System & Component Specification

This specification establishes the frontend design system, component hierarchy, and interaction guidelines for the Jockey desktop application and its phone companion. It synthesizes native desktop architecture lessons from `brewui` (SwiftUI desktop patterns) and agent UI engineering from `deepseek-harness` (AI Agent Web/Desktop architecture).

---

## 0. Working on UI (read first)

1. **Look before you build.** Run `pnpm dev` and open `http://localhost:1420/#catalog` (dev only, `src/components/catalog/Catalog.tsx`). It renders every primitive, the session-status vocabulary and the adaptive-layout frames in the current theme; toggle themes with ⌘⇧L.
2. **Compose, don't restyle.** A view is primitives plus layout. If the catalog lacks what you need, add a primitive to `src/components/ui/`, export it from `ui/index.ts`, and add it to the catalog in the same change.
3. **Check both themes and both shells.** Dark is the default and light must stay equivalent (every `--ui-*` token is defined for both). Desktop and the phone companion share components, so check a phone width (390px) and a desktop pane.
4. **`pnpm lint:ui` must pass.** It runs before every `pnpm build` (`scripts/check-ui.mjs`):
   - Hard rules, zero allowed anywhere in `src/`: Tailwind palette colour classes (`text-blue-400`, `bg-amber-500/15`, ...) and raw hex colours in markup.
   - Ratchet: raw `<button>` outside `components/ui/` may only decrease per file (`scripts/ui-baseline.json`). After removing some, run `pnpm lint:ui:update` to lock the lower count in. Never raise a count to make a check pass.

## 1. Core Design Principles

### SolidJS Rendering Boundary
Jockey uses SolidJS rather than React. Component APIs should expose accessors and narrow event callbacks, while session state remains owned by the session manager. Streaming updates must mutate the smallest relevant store path with `produce`; a provider event must not cause the whole application or all sessions to re-render.

Agent output follows this boundary:

```text
AgentEventEnvelope -> event bus -> session update -> AgentBlockRenderer -> UI primitive
```

`AgentBlockRenderer` is the common block boundary for text, thought, tool, image, error, and unknown/other blocks. `CapabilityGate` controls optional surfaces such as usage, terminal detail, plan, and structured input. Unsupported capability means hidden or explicitly unavailable—not an empty card with fabricated values.

### Catalog-First Rule
Never handcraft ad-hoc controls (buttons, badges, inputs, dialogs, checkboxes) inside individual views or tab components. Always check `src/components/ui/` first. If two features require a shared interactive pattern, promote it to `src/components/ui/`.

### Strict Semantic Tokens
Hardcoded Tailwind utility colors (such as `text-blue-500`, `bg-amber-500/15`, raw hex values) are prohibited in components. All colors and borders must be derived from the `--ui-*` CSS custom properties defined in `src/index.css` or semantic tones:
- **`neutral`**: Muted metadata, inactive states, secondary chips
- **`info`**: Informational banners, active agents, primary links
- **`success`**: Completed tasks, clean diffs, connected status
- **`warning`**: Pending review, caveats, uncommitted changes
- **`danger`**: Errors, destructive actions, failure states

### Session Status Vocabulary
"Is this agent active?" has exactly one answer, computed once: `sessionStatus()` in `src/lib/sessionStatus.ts` maps `{ running, submitting, externalActive, error, pendingPermissions, pendingUserInputs }` to one of six states, in priority order:

| Status | Label | Tone | Meaning |
| --- | --- | --- | --- |
| `needs-approval` | Approval needed | warning | a tool call waits for the user's decision |
| `needs-answer` | Answer needed | warning | the agent asked the user a question |
| `error` | Agent error | danger | the last turn failed and nothing is running |
| `working` | Working | info (animated) | a turn is running in Jockey |
| `external` | Active in CLI | info (animated) | the conversation is being driven from a terminal |
| `idle` | Idle | neutral | waiting for input |

Render it only through `StatusBadge` (dot + label; `quiet` hides idle) or `StatusDot` (dense rows such as tabs). Never re-derive status from flags in a view or invent a new label; extend the model and `SESSION_STATUS_LOOK` instead.

### Theme
`src/lib/theme.ts` + `src/lib/useTheme.ts`, shared by desktop and companion. The stored preference is `dark | light | system`; `system` follows `prefers-color-scheme` live. Quick switch: the `ThemeToggle` primitive (desktop topbar, companion headers, catalog) and ⌘⇧L on desktop; Settings → Appearance offers the same three choices. `index.html` and `src/companion/index.html` resolve the stored preference before first paint, so do not set `data-theme` anywhere else. Every token must be defined for both themes; a token added to one theme block only is a bug (that is how all state tones once rendered uncoloured in dark).

### Responsive & Multi-Shell Adaptation
- **Container queries, not viewport media queries**, for component layout. A desktop pane can be as narrow as a phone; name a container (`container: control-center / inline-size`) and adapt with `@container control-center (max-width: 600px)`. Keep `@media` for true device concerns (safe areas, hover capability, `prefers-*`).
- **Touch**: interactive targets are at least 44×44px in the companion; text inputs use 16px font so iOS does not zoom.
- **No hover-only affordances.** Anything revealed on hover (row actions, delete) must also be reachable by tap/long-press or always visible in the companion.
- **Long content never widens the page.** Containers get `min-width: 0`; commands, paths and code wrap (`overflow-wrap: anywhere`) or scroll inside their own box.

### Interaction Guidelines
- Every async action shows feedback within 100ms (disabled state + verb-ing label, e.g. "Sending…"), and a failure keeps the user's input (drafts, form values).
- Destructive actions confirm inline or via dialog and state what will be affected (e.g. role deletion lists the open sessions using the role).
- Navigation toggles live in chrome (topbar / companion header), not in strips inside the content area.
- Keyboard: ⌘K new session, ⌘, settings, ⌘B left sidebar, ⌘⇧B right dock, ⌘1/2/3 files/git/terminal, ⌘⇧M archived sessions, ⌘⇧L theme. Show the shortcut in the control's tooltip.
- Status, decisions and errors are reachable from the Control Center on both shells; a view must not be the only place a pending decision is visible.

### Micro-Primitive Hierarchy
Distinguish strictly between the three visual micro-indicators:
1. **`Badge`** (`src/components/ui/badge.tsx`): 18px compact, non-interactive tag for categorical labels, status tags, and roles (`subtle | outline | solid`).
2. **`Pill`** (`src/components/ui/pill.tsx`): 24px capsule chip with optional click handler, active states, and remove triggers for filters and session selectors.
3. **`StateDot`** (`src/components/ui/state-dot.tsx`): 5-state status dot (`idle`, `done`, `warning`, `error`, `ongoing`). For `ongoing`, renders an SVG 3×3 matrix pulse animation instead of an abrupt CSS spinner. For sessions use `StatusDot`/`StatusBadge`, which wrap it with the status vocabulary.

### Passive Views & Mutually Exclusive Load States
Views and tabs must remain passive: they bind state and forward user events to services/models. Avoid scattering boolean flags (`isLoading`, `hasError`, `isEmpty`); use the mutually exclusive `AsyncContent` container (`loading | error | empty | ready`).

### Extension Safety
Tool and block variants use keyed registries. Keys are normalized and duplicate registrations at the same priority fail fast; a higher-priority registration must be explicit. Custom renderers are isolated by a Solid `ErrorBoundary`, so malformed provider data produces one error card rather than breaking the message window.

---

## 2. Component Catalog (`src/components/ui/`)

### 2.1 Buttons & Actions
- **`Button`**: Accessible button with variants (`primary`/`default`, `secondary`, `outline`, `ghost`, `danger`/`destructive`) and sizes (`sm`, `md`, `lg`). One `primary` per surface.
- **`IconButton`**: Square 1:1 icon button for toolbars and table rows. Always pass `title` and `aria-label`.
- **`ToolbarButton`**: Chrome toolbar icon button with an `active` state (topbar, dock headers).
- **`RowButton`**: Full-width clickable list row.
- **`ThemeToggle`**: Cycles dark → light → system; the icon shows the current choice.
- **`createCopyFeedback`**: Reactive hook returning `copied()` signal that auto-resets after 1.2s. Use this for all copy buttons across user and agent messages.

### 2.2 Form & Input Controls
- **`Input`**: Single-line text input with `error` and `monospace` props.
- **`Textarea`**: Multi-line auto/fixed input with `monospace` and `error` states.
- **`FormField`**: Standard field wrapper with `label`, `hint`, `required` asterisk, `description`, and `error` callout.
- **`Checkbox`**: Accessible Kobalte checkbox (`@kobalte/core/checkbox`) supporting `checked`, `indeterminate`, label, and description.

### 2.3 Feedback & State
- **`Alert`**: Semantic notification banner supporting 5 tones (`neutral`, `info`, `success`, `warning`, `danger`), title, children body, action button, and dismiss button.
- **`EmptyState`**: Centered empty container with icon/emoji, title, description, and action button.
- **`AsyncContent`**: High-order switch container for `loading`, `error` (with Retry), `empty`, and `ready` states.
- **`StatusBadge`** / **`StatusDot`**: Session status from `lib/sessionStatus.ts` (see Section 1).

### 2.4 Modals & Destruction Confirmation
- **`Dialog`**, **`DialogContent`**, **`DialogBody`**, **`DialogFooter`**: Standardized accessible modal system powered by `@kobalte/core/dialog`.
- **`RiskConfirmation`**: Destructive confirmation modal requiring explicit checkbox acknowledgement before enabling the confirm/delete button. Use for deleting sessions, roles, or database reset.

### 2.5 Master-Detail Layouts
- **`MasterDetailView`**: Split-pane layout container for settings, management tabs, rules, and skill registries.
- **`MasterPane`**: Left list pane with search filtering and "New" action header.
- **`MasterItem`**: Clickable item with title, subtitle, active highlight, and hover delete button.
- **`DetailPane`**: Right detail workspace with header, action buttons, error banner, and empty state fallback.

### 2.6 Navigation, Menus & Panels
- **`Tabs`**, **`TabsList`**, **`TabsTrigger`**, **`TabsContent`**: Tabbed navigation. **`SegmentedControl`** / **`SegmentButton`** cover compact mutually exclusive toggles.
- **`Switch`**: On/off setting control.
- **`Tooltip`** / **`TooltipContent`**: Hover/focus label with a `placement` prop.
- **`DropdownMenu`** with `DropdownTrigger`, `DropdownContent`, `DropdownItem`, `DropdownSub*`, `DropdownLabel`, `DropdownDescription`, `DropdownSeparator`: Trigger-anchored menus.
- **`ContextMenuSurface`**, **`ContextMenuItem`**, **`ContextMenuSeparator`**: Right-click menus.
- **`Panel`**, **`PanelHeader`**, **`PanelBody`**, **`PanelHeaderAction`**: Base panel shell.
- **`DockPanel`** with `DockPanelToolbar`, `DockPanelBody`, `DockTabStrip`: Full-height shell for the left/right tool docks, built on `Panel`.
- **`PopupSelectView`**: Floating, searchable list card above the composer, used by the `popupSelect` command UI kind (see Section 5).

### 2.7 Agent Output Truncation
- **`headTailCap(content, maxLines, headRatio)`**: Geometric truncation utility for terminal outputs, diffs, and agent tool execution outputs. Keeps the first $N$ and last $M$ lines with a collapsed summary indicator in the middle.

---

### 2.8 Provider Event Projection
- **Transparent protocol metadata**: Known lifecycle, rate-limit, and aggregate-diff frames are consumed by the adapter and do not render as message cards.
- **Unknown event fallback**: A genuinely unknown provider frame remains lossless, but adjacent unknown frames render as one collapsed `ProviderEventGroup` with the raw payload available on demand.
- **Execution group**: A `ToolCallGroup` may contain ordered tool calls, thought/process text, and provider-event groups before, between, or after calls. This is one execution container; individual tool rows still use the existing `toolCallId`-based grouping and detail presentation.
- **Message boundary**: Normal assistant `TextDelta`, final reply text, images, permission requests, and plan updates cut the execution group. Providers that stream explanatory text before the final reply must classify it as message text unless the adapter explicitly marks it as process/thought content.
- **Rendering rule**: UI components never inspect Claude/Codex/Pi wire fields to decide whether an event is visible. That policy belongs to the adapter/compatibility projection layer.

## 3. Agent Protocols

Every runtime is reached through an external ACP adapter; provider wire formats never reach the UI. Protocol rules live in the repository guidelines (`CLAUDE.md`, "ACP Protocol Standards") and `docs/architecture/runtime-adapters.md`.

## 4. Streaming and Performance Contract

1. The event bus validates `sessionId`, `turnId`, `seq`, and `runToken` before touching session state.
2. Text/thought deltas are coalesced before expensive markdown or layout work; tool updates use id-based upsert and tool output uses append-only deltas.
3. Scroll-to-bottom is scheduled through one RAF per session view and is skipped when the user has scrolled away from the bottom.
4. Historical messages are rendered through a bounded window (`MESSAGE_RENDER_WINDOW`), and off-screen rows use `content-visibility: auto` (`index.css`, `.message-list-inner`; the last rows are always rendered so scroll-to-bottom measures real heights). Do not add virtualization until profiling shows this is insufficient.
5. Avoid `transition-all`, repeated markdown parsing, deep cloning of `AppSession`, and provider JSON parsing in components.
6. Preserve `null` for unreported usage fields and preserve unknown payloads for diagnostics.

The performance target is interaction stability during a sustained stream, not a particular framework benchmark: the composer remains responsive, tool cards update in place, and a late/cancelled run cannot overwrite a newer run.

## 5. Component Ownership

| Component/layer | Owns | Must not own |
| --- | --- | --- |
| `acpEventBus` | event validation and routing | provider-specific rendering decisions |
| `SessionManager` | session store, bounded updates, scroll lifecycle | adapter protocol parsing |
| `AgentBlockRenderer` | block-to-view dispatch and error isolation | queueing or runtime selection |
| `ToolCallGroup` | tool call presentation, terminal/diff affordances | alias detection by substring |
| `CapabilityGate` | optional surface visibility | capability discovery or mutation |
| `ui/*` primitives | accessible visual/control semantics | agent protocol knowledge |
| `lib/sessionStatus.ts` | the session status vocabulary and its priority | presentation (labels/tones live in `ui/status-badge.tsx`) |
| `lib/theme.ts`, `lib/useTheme.ts` | theme preference, resolution and painting | per-view theme overrides |
| `engineEvents` (`src/lib/engineEvents.ts`) | opening and settling live turns from `turn/started` / `turn/finished`, syncing queues from `queue/changed`, adopting messages from `message/added` and `turn/finished`, patching session lists from `session/changed` | persisting anything; the engine owns every durable fact |
| Command UI registry (`src/lib/commandUi/registry.ts`) | composer command shells (`popupSelect`, `action`) registered against `contract.ts` specs | command parsing or turn submission (the engine routes input) |
