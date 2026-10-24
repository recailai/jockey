# Runtime adapter architecture

Jockey is an ACP client and a project/session/decision control center. All live agents use ACP over stdio. Provider-specific conversion is maintained by external adapters, not Jockey.

## Versions and launch boundary

| Agent | Runtime key | Adapter |
| --- | --- | --- |
| Claude | `claude-code` | `@agentclientprotocol/claude-agent-acp@0.94.0` |
| Codex | `codex-cli` | `@agentclientprotocol/codex-acp@2.2.2` |
| Pi | `pi-cli` | `pi-acp@0.0.34` |
| Antigravity | `antigravity-cli` | Google `agy_acp_server.par` 1.4.1 |

The Rust SDK is pinned to `agent-client-protocol =3.5.0`. The client negotiates stable ACP v1; this does not opt into draft protocol v2. Distribution details are from the [official ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json).

Node adapters are installed under the application data directory with exact versions. An existing executable is insufficient: the installed package version must match. Install attempts and manifests are scoped to each adapter. Catalog detection checks installation metadata/PATH without starting agent conversations or running adapter `--version` commands. Actual installation happens on demand. Antigravity uses the official Google executable in `adapters/bin/` or PATH. Custom ACP profiles remain supported.

## Responsibilities

Desktop IPC, the HTTP/SSE companion and Jockey MCP submit to the same engine. The engine owns durable messages, session routing, turn reservation, input queues, cancellation and lifecycle events. Views render engine state. The shared control center renders pending permissions directly on cards, allows explicit one-time batch approval and links to the existing conversation UI.

`acp/adapter.rs` resolves the external executable and launch environment. `acp/session/cold_start.rs` initializes ACP and creates or loads a session. `acp/transport.rs` correlates JSON-RPC requests and provides backpressure, cancellation and stderr diagnostics. `acp/client.rs` handles standard ACP notifications, permissions, filesystem and terminal callbacks. `acp/worker/` owns process slots and session lifetime. `adapter_runtime` dispatches only ACP or the offline mock; there are no native/headless provider runners.

Slots are scoped by application session, runtime and persona. Concurrent sessions are independent; input within one session is serial. ACP `session/cancel` interrupts a turn; the engine can cancel and drain before delivering queued input. Native steer and next-step controls are no longer advertised.

Startup opens the control center. Viewing its summaries does not create agent sessions. Model/mode/configuration discovery is requested when the user opens the relevant chat or role editor, and fresh catalog results can be reused.

## Recovery and imports

The transcript importers remain: reading historical CLI files is separate from maintaining a live native protocol. Claude, Codex and Pi imports retain their provider IDs so ACP adapters can load those conversations.

Schema v17 canonicalizes old Claude runtime bindings, copies the active provider handle and per-runtime configuration into the ACP slot, and preserves legacy configuration snapshots and chat messages. Old runtime/profile spellings are compatibility aliases. They never select a native execution path.

The client does not record startup announcements or `session/load` history replay as a new turn's response. Configuration and command updates still apply during setup. Failed `session/load`, or a missing load capability for an existing handle, returns an explicit error while preserving the transcript and handle. A replacement conversation requires an explicit context reset.

Fork uses ACP `session/fork` (`unstable_session_fork`) and is offered only for runtimes whose `initialize` advertised `sessionCapabilities.fork` (Claude, Codex). The engine forks the provider session first and only then creates the Jockey session, copies the transcript and binds the new handle (`engine/fork.rs`); a failed fork creates nothing. Rewind is not advertised.

Session discovery for import combines two sources (`src-tauri/src/importer/`):

- **Transcript scanners** (Claude, Codex, Pi) decide which conversations exist and supply history and turn counts. They skip probe, empty and sub-agent transcripts (Codex guardian rollouts share the parent's `session_id`, so threads are keyed by `id`), and a git worktree counts as part of the project whose checkout its `.git` file points to.
- **ACP `session/list`** (`acp/session/list.rs`), negotiated through `sessionCapabilities.list`, runs only when the import dialog refreshes its catalog and only for adapters already installed (it never installs). It supplies Claude's generated titles and the full list for runtimes without a transcript reader (Antigravity, DeepSeek Harness; dsh lists no titles or times, so its last activity comes from the session directory's newest file). Codex and Pi list the raw first message as the title, so their scanner titles are kept. Importing an ACP-only conversation binds it without history; the next turn resumes it with `session/load`.

ACP cannot replace the scanners: it has no transcript read (only `session/load`, which resumes and replays without timestamps), no per-session change notification for the external-sync watcher, no turn counts, and adapter filters differ (Claude matches the exact cwd; codex-acp lists only the configured model provider's threads from its state DB).

## Unstable ACP surfaces

`agent-client-protocol` is built with these unstable features; each is used only when the agent advertises it, and protocol v2 stays off:

| Feature | Use |
| --- | --- |
| `unstable_subagents` | The client declares `subagents`. A child announced by `subagent_update` becomes a tool row `subagent:<child id>` (kind `agent`); the child's text streams into that row's output and its tool calls nest under it via `parentId` (`acp/subagents.rs`). Child mode/config/plan updates are ignored. A running row can be stopped with `session/cancel` on the child id (`cancel_acp_subagent`). Claude hides its Agent/Task tool calls once this is declared, so the row is the only trace of a sub-agent. |
| `unstable_plan_operations` | The client declares `plan`. `plan_update` items map to the plan panel; Markdown and file plans render as one document; `plan_removed` clears the panel. |
| `unstable_end_turn_token_usage` | `PromptResponse.usage` adds input/output/cache/reasoning tokens to session usage (context occupancy still comes from `usage_update`). |
| `unstable_session_system_prompt` | A role's `systemPrompt` is sent in `append` mode on `session/new`, `load` and `resume`. |
| `unstable_session_fork` | See above. |
| `unstable_llm_providers` | Settings → Agent accounts → Model endpoint. Saved provider configs are sent with `providers/set` after every `initialize`; header values live in the OS keychain, SQLite keeps only names. Changing a config evicts warm connections. |
| `unstable_mcp_over_acp` | Compiled in only; no adapter supports MCP over ACP yet. |

An update kind the schema does not know (for example Claude's AIR-only `async_task_*`) is logged and skipped instead of closing the connection.

## Validation

Unit tests cover runtime routing, pinned-version detection, data migration with an existing ACP slot, transcript/handle preservation, suppression of setup replay, permission acknowledgements and cancellation isolation. Shared HTTP contract and production CSS checks cover the companion boundary. Real adapter smoke checks exercise standard initialize/new/prompt operations without tools or file modifications; authentication-dependent failures must be reported separately from protocol support.


### DeepSeek Harness (`dsh-cli`)

DeepSeek Harness speaks ACP itself (`dsh --profile acp`), so there is no adapter package: Jockey resolves the user's `dsh` from the shell PATH and does not pin a version. Observed with dsh 0.2.1-rc.2:

- `sessionCapabilities` = `list`, `resume`, `close`; no `loadSession`. Resuming a stored handle uses `session/resume` (no replay); the transcript Jockey recorded is the history.
- MCP over HTTP only, which the Jockey MCP bridge uses, so the bridge stays injected (unlike Codex/Pi/Antigravity).
- No image prompts.
- Config options: `model` is grouped by provider and its values are JSON-encoded `[provider, model]` pairs; `reasoning_effort` (`thought_level`) is off/low/high/max. No modes.
- It writes a plain-text log line to stdout while listing models; the transport skips stdout lines that cannot start a JSON-RPC frame (`parse_frame`) instead of dropping the connection.
