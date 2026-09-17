# Jockey UI Design System & Component Specification

This specification establishes the frontend design system, component hierarchy, and interaction guidelines for the Jockey desktop application. It synthesizes native desktop architecture lessons from `brewui` (SwiftUI desktop patterns) and agent UI engineering from `deepseek-harness` (AI Agent Web/Desktop architecture).

---

## 1. Core Design Principles

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

## 3. Native Agent Protocols (`src-tauri/`)

### Pi RPC Protocol Guidelines
- **Fire-and-forget UI updates**: Messages with `type: "extension_ui_request"` and methods `setStatus`, `notify`, `setWidget` must NOT receive synthetic JSON-RPC responses.
- **Interactive UI requests**: Requests with method `confirm` or `select` must be answered with `type: "extension_ui_response"`:
  ```json
  { "type": "extension_ui_response", "id": "req_1", "confirmed": true }
  ```
- **Session ID extraction**: In `extract_pi_session_id`, always prioritize `sessionId` / `session_id` (UUID or slug) over `sessionFile` / `session_file` (absolute disk path) to prevent database pollution.
- **Turn cancellation**: Soft-abort via `{"type": "abort"}` over stdin to allow the Pi runtime to settle cleanly without killing the child process.
