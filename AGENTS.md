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
