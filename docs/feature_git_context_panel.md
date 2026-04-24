# Feature Request — Git Context Panel

> **Status**: Requirement captured, not yet designed
> **Opened**: 2026-04-24
> **Owner**: TBD

## Summary

Inside a chat session, the user wants to see git state for the session's `cwd` — branch name, modified/untracked/staged files, and diffs for selected files or folders — without leaving the app. Today the user has to drop to a terminal (`git status`, `git diff`) to answer "what am I about to talk to the agent about?".

## Motivation

1. **Grounding the conversation**. Roles routinely operate on the working tree (code review, refactor, commit-message drafting). Seeing the diff inline lets the user verify the agent is reasoning about the right changes.
2. **Picking files to `@mention`**. Commit 77a9b03 shipped fuzzy path completion for `@mentions`; a git panel is the natural next step — the user thinks "I want the agent to look at the files I just changed", not "I want to type `@src/foo.rs`".
3. **Reviewing agent edits**. When an agent (Claude Code, Codex) writes to the working tree via `writeTextFile`, the panel shows the resulting diff in the same window.

## Scope (v1)

**In scope**:
- A collapsible side panel, toggleable per-session, scoped to the session's `cwd` (which already varies per-session — see commit 77a9b03).
- **Branch header**: current branch, detached-HEAD state, ahead/behind vs. upstream, dirty flag. Click to refresh.
- **File list**: three groups — `Staged`, `Unstaged`, `Untracked`. Each row shows path + status letter (`M`/`A`/`D`/`R`/`??`). Folder rollup when a subdirectory has >N entries.
- **Diff viewer**: clicking a file shows unified diff (staged vs. working tree by default; toggle for "vs. HEAD"). Clicking a folder shows the aggregated diff for files under it.
- **Add-to-prompt button**: per file / per folder — inserts the path (not the full diff) into the chat input as a `@mention`. The agent reads the file via `readTextFile` through ACP — no need to stuff diffs into the prompt.

**Explicitly out of scope for v1**:
- Staging / unstaging / committing from the UI. Users `git add` in their terminal; we read-only.
- Branch switching, merge/rebase, remote ops. Same reason.
- Per-hunk selection, word-level diff. Start with whole-file unified diff; refine later.
- Non-git VCS (hg, jj, svn). Defer.
- Binary file diffs. Show "binary file changed", no viewer.
- Large-repo perf tuning (shallow clones, partial checkouts). Out of scope until we hit it.

## User-visible surface

Three entry points:

1. **Sidebar toggle** in the main chat view — a new icon next to the existing `@mention`/role picker.
2. **Slash command** — `/git` opens the panel; `/git diff <path>` inserts a hint into the chat input.
3. **Keyboard shortcut** — TBD; something that doesn't collide with `Cmd/Ctrl+F` (in-session search, from commit 525ecb1).

Panel states:
- **Not a git repo**: panel shows "Not a git repository" with the cwd path. Offer `git init` link? (v1: no.)
- **Clean working tree**: branch header + "Working tree clean".
- **Dirty tree**: the three groups described above.
- **Git command missing on PATH**: panel shows a one-time install hint, then collapses.

## Non-goals (behavioral)

- **No background polling**. Refresh on explicit user action (panel open, click refresh), on focus-regain, and after a role's `writeTextFile` completes (this one matters — agent edits should visibly update the panel).
- **No git state in SQLite**. The working tree is ground truth; we don't cache anything durable.
- **No write operations driven by the panel**. If users want the agent to commit, they ask the agent in chat; Claude Code already does this via shell tools.

## Open questions

- Which git invocation strategy:
  - (A) Shell out to `git` binary via `tokio::process::Command`. Simplest, matches user's env (signing, hooks).
  - (B) Use a Rust crate (`git2` / `gix`). Faster, no fork per call. Larger binary, FFI risk on Windows/musl.
  - Initial preference: **(A)** for v1 — we don't need speed and users already have `git` installed if they're in a repo. Revisit if the panel feels sluggish.
- Diff viewer rendering: use an existing component (`diff2html` via JS, `similar-asserts` via Rust + server-side render) or hand-roll on SolidJS? Preference: whatever composes with the current chat view's virtual-list approach.
- Folder rollup threshold (the "when do we show one folder entry instead of N file entries" cutoff). Tentative: >10 children.
- Scope of "aggregated folder diff": concatenate unified diffs (`git diff -- folder/`) or show a file-list + click-through? Concatenating is simpler; lean that way.
- Session cwd can change via `/cd` (commit 77a9b03). Panel should re-read on cwd change. Anything special needed for symlinks / worktrees / submodules? Flag for design review, punt if not trivial.

## Relationship to existing work

- **Session cwd** (commit 77a9b03): panel is scoped by the same cwd that drives `@mention` completion. Re-use `get_app_session_cwd` in `src-tauri/src/db/app_session.rs`.
- **`@mention` completion** (commit 77a9b03): panel's "add to prompt" inserts the same `@path` form the completion already understands. No new syntax.
- **File-like mention fix** (commit 8f9533b): paths from the panel should already route as file refs, not role lookups.
- **Message window search** (commit 525ecb1): a future panel extension might let the user search inside diffs; for v1 we rely on the browser's native in-element find.

## Rough implementation skeleton (to seed design)

- `src-tauri/src/git/` — new module.
  - `status.rs` — `fn status(cwd: &Path) -> Result<GitStatus, GitError>` (branch info + file groups).
  - `diff.rs` — `fn diff_file(cwd, path, vs_head: bool) -> Result<String, GitError>`; `fn diff_dir(cwd, path, vs_head) -> Result<String, GitError>`.
  - `error.rs` — `enum GitError { NotARepo, GitNotFound, CommandFailed(String) }`.
- `src-tauri/src/commands/git_cmd.rs` — three Tauri commands: `git_status_cmd`, `git_diff_cmd`, `git_repo_info_cmd`. All take `app_session_id` and resolve cwd via existing helper.
- `src/components/GitPanel.tsx` (or wherever we put new UI; App.tsx is currently monolithic per CLAUDE.md). Renders branch header + 3 groups + diff viewer.
- No new SQLite tables. No background tasks.

This skeleton is illustrative; the actual design doc should firm up the invocation strategy and diff rendering choice before code lands.

## Follow-ups this unlocks

- **Agent-initiated staging / commit drafting**: the panel state is the signal the agent needs to suggest "shall I draft a commit for these changes?". Out of v1, but the data shape below (staged/unstaged lists) is exactly the input.
- **Diff-aware summaries**: a forked session (see `rfd_session_fork.mdx`) that takes `git diff` as initial context and produces a PR description without polluting the main history.
- **Workflow hooks**: a workflow step that fails if the working tree isn't clean, or that auto-captures the diff into a role's context.

## Links

- `src-tauri/src/db/app_session.rs` — `get_app_session_cwd` (existing)
- `src/lib/tauriApi.ts` — where new `invoke()` wrappers go
- `docs/rfd_session_fork.mdx` — related, for future diff-aware summaries
- `docs/feature_git_context_panel.md` — this file
