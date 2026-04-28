# Jockey MVP UI Shell Rebuild

> Status: Second-pass implementation in progress
> Updated: 2026-04-28

## Summary

Jockey is still in MVP stage, so the UI should not preserve old shell compatibility. The current direction is a direct Solid-UI/shadcn-inspired rebuild: keep the AppSession, role, workflow, terminal, git, and preview business logic, but replace the visual shell, tokens, and primitives with one compact desktop design system.

The product information architecture remains Jockey-specific:

- `AppSession` is the workspace object, not `Project`.
- Workflows are labeled `Automations` in the UI.
- Files and diffs stay in the wide main preview area.
- Terminal stays in the right tool panel.
- Settings is a full page.

## Implementation Direction

- `AppShell` owns the workspace layout: sidebar, header, chat stack, composer, right tool dock, settings surface, and toast layer.
- `SessionSidebar`, `WorkspaceHeader`, and `ToolPanelDock` are now the public shell names.
- `src/components/ui/*` is the only primitive layer. Product components should use `Button`, `IconButton`, `ToolbarButton`, `RowButton`, `ListRow`, `SplitButton`, `Input`, `Textarea`, `Switch`, `Dropdown`, `Dialog`, `Panel`, `Badge`, `Tabs`, `SegmentedControl`, and `EmptyState`.
- Kobalte is used only inside UI primitive wrappers. Solid-UI/shadcn is a visual and structural reference, not an installed framework.

## Visual Rules

- Theme values enter through `--ui-*` tokens only.
- Default light theme is white and neutral: main `#ffffff`, sidebar `#f6f6f7`, border `#e4e4e7`, text `#18181b`, muted `#71717a`, primary near-black.
- Dark theme is neutral gray-black, not blue/purple.
- Standard heights: rows and toolbar controls `32px`, compact controls `30px`, topbar `48px`, right panel header `46px`.
- Standard radius: buttons `8px`, rows `10px`, panels/dialogs/preview/composer `14-18px`.
- AppSession rows, Settings rows, right-panel rows, and toolbar controls must share the same vertical centering and hover/focus behavior.

## Cleanup Rules

- Remove obsolete UI references to `ActivityBar`, `SessionTabs`, `ConfigDrawer`, `ManagementPanel`, and VS Code-style side panels when their behavior has moved into the new shell.
- Do not keep parallel CSS systems such as old `.icon-btn`, `.ui-toolbar-button`, `.settings-switch`, or one-off management utility styles once a `jui-*` primitive covers the same role.
- Do not add Hope UI or a second theme system.
- Do not custom-draw macOS traffic lights; keep Tauri native overlay traffic lights and align controls from chrome tokens.

## Current Migration State

- Main shell slots are in `AppShell`.
- Sidebar/header file names have been migrated to `SessionSidebar` and `WorkspaceHeader`.
- Run and IDE controls use the shared split-button primitive.
- Files/Git/Terminal panels use shared `Panel` structure.
- Preview mode tabs use `SegmentedControl`.
- Permission, session error, quick-add MCP, and tool-call surfaces have started moving to `Button`, `Panel`, `Badge`, `Switch`, and semantic state tokens.

Remaining cleanup:

- Reduce legacy `theme-*` helper usage in management subpages.
- Replace the remaining custom branch picker with a full dropdown/combobox primitive when a searchable select primitive is added.
- Collapse duplicate CSS token blocks into a single ordered token section.

## Verification

- `pnpm exec tsc --noEmit`
- `pnpm build`
- Manual UI checks:
  - AppSession create, switch, close, rename.
  - Role switcher, run action, IDE open, and Git actions.
  - Files/Git/Terminal right panel open, close, resize.
  - Preview tabs and file/diff content remain wide in the main area.
  - Settings General/Appearance/Configuration/Automations/Roles do not visually split from the main shell.
  - Light and dark themes use the same component density, border strength, radius, and focus rings.
