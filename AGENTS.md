# Repository Guidelines

## Project Structure & Module Organization
- `src/`: SolidJS + TypeScript UI (`App.tsx` is the main view entry, `index.tsx` bootstraps app, `index.css` holds Tailwind/CSS variables).
- `src-tauri/src/`: Rust backend for Tauri commands and orchestration logic (`lib.rs` and `acp.rs`).
- `docs/`: Architecture notes and RFC-style design docs.
- `public/`: Static assets packaged by Vite.
- Generated output: `dist/` (frontend build) and `src-tauri/target/` (Rust build artifacts); do not commit these.

## Build, Test, and Development Commands
- `pnpm dev`: Run Vite dev server for frontend iteration.
- `pnpm tauri dev`: Run the full desktop app (frontend + Rust backend hot reload).
- `pnpm build`: Build production frontend assets into `dist/`.
- `pnpm tauri build`: Build distributable desktop bundles.
- `cargo check --manifest-path src-tauri/Cargo.toml`: Fast Rust type check.
- `cargo test --manifest-path src-tauri/Cargo.toml`: Run Rust tests.
- `cargo clippy --manifest-path src-tauri/Cargo.toml`: Rust lint checks.
- `cargo fmt --manifest-path src-tauri/Cargo.toml`: Format Rust code.

## Coding Style & Naming Conventions
- TypeScript/Solid: use 2-space indentation, semicolons, `camelCase` for variables/functions, `PascalCase` for components and type names.
- Rust: rely on `rustfmt` defaults (4-space indentation), `snake_case` for functions/variables, `CamelCase` for structs/enums.
- Keep Tauri command payloads and frontend types in `camelCase` to match existing serde usage (`#[serde(rename_all = "camelCase")]`).
- Prefer small, focused modules; place protocol/backend logic in `src-tauri/src/` and UI behavior in `src/`.
- **UI Design System (Catalog-First)**:
  - Do NOT craft ad-hoc buttons, badges, inputs, or dialogs. Always consume primitives from `src/components/ui/` (`Badge`, `Pill`, `StateDot`, `Button`, `FormField`, `Checkbox`, `Alert`, `Dialog`, `MasterDetailView`, `AsyncContent`).
  - Strict semantic tokens: Do NOT use hardcoded colors (e.g. `text-blue-500`, `bg-amber-500/15`). Rely on `--ui-*` tokens or semantic tone properties (`neutral`, `info`, `success`, `warning`, `danger`).
  - Read [docs/ui_design_and_component_specification.md](file:///Users/sexy/Documents/GitHub/jockey/docs/ui_design_and_component_specification.md) for detailed specifications.
- **Native Protocol Standards**:
  - For `PiRpc`, fire-and-forget UI updates (`setStatus`, `notify`, `setWidget`) must NOT be answered with synthetic JSON-RPC results.
  - Interactive UI requests in Pi must be answered with `{"type": "extension_ui_response", ...}`.
  - In `extract_pi_session_id`, always prioritize clean `sessionId` over `sessionFile` paths.

## Testing Guidelines
- Primary automated tests are currently Rust-side via `cargo test`.
- Add unit tests next to Rust modules when introducing non-trivial logic.
- Frontend test framework is not configured yet; include manual verification steps in PRs (for example: team creation, role binding, assistant chat flow).

## Commit & Pull Request Guidelines
- Follow Conventional Commits, as seen in history: `feat: ...`, `docs: ...`, `chore: ...`.
- Keep commits scoped to one change theme; use imperative, specific summaries.
- PRs should include:
  - concise problem/solution description,
  - linked issue (if available),
  - testing evidence (`cargo test`, `pnpm build`, manual UI checks),
  - screenshots or short recordings for UI changes.
