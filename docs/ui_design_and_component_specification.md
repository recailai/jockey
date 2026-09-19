# Jockey UI Design System & Component Specification

This specification establishes the frontend design system, component hierarchy, and interaction guidelines for the Jockey desktop application. It synthesizes native desktop architecture lessons from `brewui` (SwiftUI desktop patterns) and agent UI engineering from `deepseek-harness` (AI Agent Web/Desktop architecture).

---

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

### Micro-Primitive Hierarchy
Distinguish strictly between the three visual micro-indicators:
1. **`Badge`** (`src/components/ui/badge.tsx`): 18px compact, non-interactive tag for categorical labels, status tags, and roles (`subtle | outline | solid`).
2. **`Pill`** (`src/components/ui/pill.tsx`): 24px capsule chip with optional click handler, active states, and remove triggers for filters and session selectors.
3. **`StateDot`** (`src/components/ui/state-dot.tsx`): 5-state status dot (`idle`, `done`, `warning`, `error`, `ongoing`). For `ongoing`, renders an SVG 3×3 matrix pulse animation instead of an abrupt CSS spinner.

### Passive Views & Mutually Exclusive Load States
Views and tabs must remain passive: they bind state and forward user events to services/models. Avoid scattering boolean flags (`isLoading`, `hasError`, `isEmpty`); use the mutually exclusive `AsyncContent` container (`loading | error | empty | ready`).

### Extension Safety
Tool and block variants use keyed registries. Keys are normalized and duplicate registrations at the same priority fail fast; a higher-priority registration must be explicit. Custom renderers are isolated by a Solid `ErrorBoundary`, so malformed provider data produces one error card rather than breaking the message window.

---

## 2. Component Catalog (`src/components/ui/`)

### 2.1 Buttons & Actions
- **`Button`**: Accessible button with variants (`default`, `secondary`, `outline`, `ghost`, `destructive`) and sizes (`sm`, `md`, `lg`).
- **`IconButton`**: Square 1:1 icon button for toolbars and table rows.
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

### 2.4 Modals & Destruction Confirmation
- **`Dialog`**, **`DialogContent`**, **`DialogBody`**, **`DialogFooter`**: Standardized accessible modal system powered by `@kobalte/core/dialog`.
- **`RiskConfirmation`**: Destructive confirmation modal requiring explicit checkbox acknowledgement before enabling the confirm/delete button. Use for deleting sessions, roles, or database reset.

### 2.5 Master-Detail Layouts
- **`MasterDetailView`**: Split-pane layout container for settings, management tabs, rules, and skill registries.
- **`MasterPane`**: Left list pane with search filtering and "New" action header.
- **`MasterItem`**: Clickable item with title, subtitle, active highlight, and hover delete button.
- **`DetailPane`**: Right detail workspace with header, action buttons, error banner, and empty state fallback.

### 2.6 Agent Output Truncation
- **`headTailCap(content, maxLines, headRatio)`**: Geometric truncation utility for terminal outputs, diffs, and agent tool execution outputs. Keeps the first $N$ and last $M$ lines with a collapsed summary indicator in the middle.

---

### 2.7 Provider Event Projection
- **Transparent protocol metadata**: Known lifecycle, rate-limit, and aggregate-diff frames are consumed by the adapter and do not render as message cards.
- **Unknown event fallback**: A genuinely unknown provider frame remains lossless, but adjacent unknown frames render as one collapsed `ProviderEventGroup` with the raw payload available on demand.
- **Execution group**: A `ToolCallGroup` may contain ordered tool calls, thought/process text, and provider-event groups before, between, or after calls. This is one execution container; individual tool rows still use the existing `toolCallId`-based grouping and detail presentation.
- **Message boundary**: Normal assistant `TextDelta`, final reply text, images, permission requests, and plan updates cut the execution group. Providers that stream explanatory text before the final reply must classify it as message text unless the adapter explicitly marks it as process/thought content.
- **Rendering rule**: UI components never inspect Claude/Codex/Pi wire fields to decide whether an event is visible. That policy belongs to the adapter/compatibility projection layer.

## 3. Native Agent Protocols (`src-tauri/`)

### Pi RPC Protocol Guidelines
- **Fire-and-forget UI updates**: Messages with `type: "extension_ui_request"` and methods `setStatus`, `notify`, `setWidget` must NOT receive synthetic JSON-RPC responses.
- **Interactive UI requests**: Requests with method `confirm` or `select` must be answered with `type: "extension_ui_response"`:
  ```json
  { "type": "extension_ui_response", "id": "req_1", "confirmed": true }
  ```
- **Session ID extraction**: In `extract_pi_session_id`, always prioritize `sessionId` / `session_id` (UUID or slug) over `sessionFile` / `session_file` (absolute disk path) to prevent database pollution.
- **Turn cancellation**: Soft-abort via `{"type": "abort"}` over stdin to allow the Pi runtime to settle cleanly without killing the child process.

## 4. Streaming and Performance Contract

1. The event bus validates `sessionId`, `turnId`, `seq`, and `runToken` before touching session state.
2. Text/thought deltas are coalesced before expensive markdown or layout work; tool updates use id-based upsert and tool output uses append-only deltas.
3. Scroll-to-bottom is scheduled through one RAF per session view and is skipped when the user has scrolled away from the bottom.
4. Historical messages are rendered through a bounded window. Do not add virtualization until profiling shows the bounded window is insufficient.
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
