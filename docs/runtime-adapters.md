# Runtime adapter architecture

Jockey keeps `ACP` as the product-level runtime category. ACP is the common event and permission surface exposed to the Solid UI; it is not a requirement that every CLI implement ACP itself.

The provider field matrix and final frontend abstraction are maintained in [Agent protocol and rendering contract](agent-protocol-and-rendering.md). Update that document whenever a transport adds a field, capability, or fallback.

## Module design notes

These short notes are the local design context for each boundary. They are intentionally
prescriptive: a new adapter should fit the boundary instead of spreading another provider
branch through the application.

| Module | Background | Design principles |
| --- | --- | --- |
| `adapter_runtime` | Several process protocols share one Jockey session lifecycle. | Own probe, prompt, cancel, resume/reconnect, teardown, and declared capabilities. Never decide how a message card looks. |
| Provider normalizers | Claude, Codex, Pi, agy, and ACP expose different event names and completion rules. | Parse provider JSON once, map stable semantics, preserve unknown payloads, and keep provider field names out of Solid components. |
| Event envelope / bridge | Streaming events can arrive late, duplicated, or interleaved across roles. | Carry `sessionId`, `turnId`, `roleName`, `seq`, and `runToken`; reject stale frames before mutating session state. |
| Session reducer / store | SolidJS can update a narrow store path without rebuilding the conversation. | Keep state deterministic and replayable. `streamSegments` is ordered display state; `toolCalls` is an upsert index, not a second timeline. |
| Queue / input delivery | Providers differ between queue-only prompts, interrupt-and-replace, and native steer. | Use item ids and claim/ack/restore. Let the adapter declare delivery strategy; never clear all queued input before a send succeeds. |
| Block renderer | SAR's React components are behavior references, not components to copy into SolidJS. | Share one renderer for history and live output, use accessors and stable list boundaries, and keep unknown payloads compact and collapsed. |
| Provider event projection | DSH consumes turn/step/tool lifecycle events into durable view nodes; it does not print every transport frame as chat content. | Consume known lifecycle frames in the adapter. Keep genuine unknowns lossless, but render adjacent unknowns as one collapsed `ProviderEventGroup`; never mix them into `ToolCallGroup`. |
| Tool display registry | Tool names and payloads vary more than the visual affordance. | Match exact aliases or provider kinds first, then fall back to `other`; do not infer a category from loose substring checks. |
| Capability gates | A missing provider field is not an empty value. | Hide unsupported surfaces or show an explicit unavailable state; never synthesize empty tool, terminal, plan, or usage cards. |

The SolidJS adaptation of the SAR frontend keeps the valuable behavior—ordered blocks, tool
upserts, compact tool summaries, bounded history rendering, and error isolation—while using
fine-grained accessors and session-local mutations instead of React context or a global rerender.

Each runtime is resolved into an adapter descriptor with these fields:

- `kind` / `runtimeKey`: stable provider identity used by roles, sessions, and persisted handles.
- `binary` / `args` / `env`: the resolved launch contract after PATH and interactive-shell resolution.
- `launchMethod`: diagnostic metadata such as `native:codex-app-server`, `native:pi-rpc`, `headless-binary`, or `package-runner:pnpm`.
- `transport`: the protocol implementation selected for the runtime.
- `capabilities`: the semantic output and control capabilities the adapter can actually deliver.
- `inputDelivery`: which queue deliveries are safe for this adapter (`nextTurn`, `nextStep`, `interrupt`, `runNow`) plus the current `strategy` (`queue`, `interruptFollowUp`, or `steer`).

The descriptor is the UI-facing contract. A capability is not inferred from whether one event happened to appear during a run. `false` means the adapter cannot provide it; `undefined` is reserved for compatibility with an older profile. Unsupported controls return one stable error category, `UnsupportedCapability(<name>)`, so the UI can hide the action or show an explicit unavailable state. Adapter internals use typed `AdapterError`; public Tauri command boundaries stringify the same stable category for compatibility.

Current transport mapping:

| Runtime | Transport | Native session handle |
| --- | --- | --- |
| Claude Code | ACP bridge | ACP session id |
| Codex CLI | Codex app-server JSON-RPC | Codex thread id |
| Pi CLI | Pi JSONL RPC | `sessionId`, falling back to `sessionFile` |
| Antigravity CLI (`agy`) | headless `stream-json` (persistent when supported) | conversation id |

### Provider session persistence

`app_session_role_runtime_configs` is the durable provider-session map. Its key is
`(app_session_id, role_name, runtime_kind)`, so switching from agy to Codex and back does not
overwrite either provider's handle. The existing `acp_session_id` column is a compatibility
name for a generic provider handle; it stores the real identifier returned by the selected
runtime, not a Jockey-generated id. The primary `app_session_roles` row is only the current
runtime pointer and a compatibility mirror.

| Runtime | Value saved after a successful turn | Cold-start recovery |
| --- | --- | --- |
| Claude Code / ACP | ACP `sessionId` | `session/load` with the persisted session id |
| Claude native | Claude stream `conversation_id` | `--resume <conversation_id>` |
| Antigravity | agy `init`/`step_update`/`result` `conversation_id` | `--conversation <conversation_id>` |
| Codex | app-server `thread.id` | `thread/resume { threadId }`, falling back to `thread/start` if unavailable |
| Pi | `get_state.sessionId`, or `sessionFile` when that is the only provider value | `--session <sessionId-or-sessionFile>` |

The save path ignores blank handles, and reset clears only the selected runtime's row. A
runtime switch therefore changes the lookup key but never deletes another agent's history. The
in-memory process slot is an optimization; if it is evicted, the DB handle is sufficient for
the adapter to recover the provider session.

The protocol runners own process launch, JSONL framing, request correlation, stderr tails, cancellation, model discovery, and conversion of provider events into the existing ACP event envelope. The UI therefore does not need provider-specific event types. Each process slot is keyed by `appSessionId + runtimeKey + roleName`, so independent roles cannot share a provider session accidentally.

Model and mode discovery is runtime data, not a hardcoded model catalog. ACP providers update the runtime cache from session state; Codex refreshes through `model/list`; Pi refreshes through `get_available_models`; an explicit role-config refresh clears the previous runtime snapshot before probing again.

Native sessions reuse one long-lived provider process while the app session is active. agy 1.1+ uses the same strategy with `--input-format stream-json --output-format stream-json`: one stdin/stdout process handles multiple turns and returns one `result` event per turn. Older agy builds fall back to one-shot headless JSON when the stream flags are not advertised. If a process exits, the cwd/launch contract changes, or the session is idle for five minutes, the slot is evicted and the persisted provider handle is used for recovery: Codex via `thread/resume`, Pi via `--session <session-file>`, and agy via `--conversation <id>` when a new process is needed.

Per-tool-use permission for the native Claude provider is bridged through the `--permission-prompt-tool` contract: the runner passes a temp `--mcp-config` pointing at `current_exe --__jockey-permission-bridge`, a minimal stdio MCP server that forwards each request over a loopback socket into the shared Jockey permission UI. Requests block until the user answers, are denied on timeout, and are denied when the bridge disconnects. Roles that explicitly enable auto-approve still run with `--dangerously-skip-permissions`; without it, a session whose bridge is unavailable refuses to start instead of silently bypassing. Native MCP config mapping for agy remains future work; until then, the native runner records unmapped MCP counts in diagnostics instead of silently claiming native MCP support.

The agy stream protocol is the current documented headless integration: an `init` event is followed by `step_update` events and exactly one `result` event for each prompt. Jockey consumes the provider's `response` and `text_delta` fields, forwards tool steps into the ACP event envelope, and preserves the conversation id for the next turn. See the [official agy headless-mode documentation](https://www.agy.dev/docs/cli/headless/) for the CLI contract.

ACP itself is still actively maintained. Jockey now uses the official Rust SDK `2.0.0` with the stable `schema::v1` wire types. The SDK's high-level connection builder is intentionally not used here: the worker is a single-threaded Tokio `LocalSet` and the adapter layer needs one uniform process lifecycle, pending-request map, backpressure point, stderr tail, and shutdown path across ACP, Codex, Pi, and agy. `acp/transport.rs` therefore owns the small JSON-RPC-over-JSONL boundary while the SDK owns protocol types and serialization contracts. The old `unstable_session_model` API was removed; Claude model choices come from ACP `session/config_options`, while Codex and Pi expose their native model catalogs. The transport splits SDK 2.0 batch frames (a JSON array of JSON-RPC messages) into independently dispatched members, times out pending requests after 120 seconds, flushes every waiter on EOF, and logs orphan responses and unknown notifications for diagnosis.

All runtimes are dispatched through the `RuntimeAdapter` trait (`session/adapter_runtime.rs`), which fixes the lifecycle and control entry points — `probe`, `prompt`, `cancel`, `steer`, `discard_slot`, `reconnect_slot`, `teardown`, `reclaim_idle`, `diagnostics` — so callers (execute dispatch, session commands, worker sweeps) never branch on transport kind. The enum-backed dispatch keeps per-transport runners where the verified protocol logic lives; a fake adapter implements the same trait for the unit-test matrix.

The lifecycle adapter and semantic normalizer are separate responsibilities:

```text
RuntimeAdapter
  ├─ prompt / cancel / resume / reconnect / teardown
  ├─ descriptor and capability profile
  └─ ProviderNormalizer -> AgentEvent
```

The normalizer owns provider field names, event ordering quirks, and fallback payload preservation. It must not own session slot reuse or UI decisions. This separation lets Codex, Pi, Claude, and agy share the same Solid renderer while retaining their native resume and cancellation semantics.

Tool normalization keeps the provider's stable identifier in optional `toolName` and the
human-facing label in `title`. ACP v1 only has `title` plus `kind`; it is not treated as if it
had a hidden `name` field. Native Claude uses `tool_use.name`, Pi uses `toolName`, Codex uses
the item-specific tool/name fields, and agy uses `tool_name`/`tool_info.name`. Missing fields
stay missing, and a patch with no input/output does not clear an earlier value.

### Verified provider field mapping

The following are the fields the normalizers currently consume. They are intentionally kept in
Rust; Solid components only see the canonical event fields.

| Adapter | Provider frames | Canonical mapping | Important absence rule |
| --- | --- | --- | --- |
| Claude stream-json | `stream_event.content_block_start` with `content_block.type=tool_use`, `id`, `name`; `content_block_delta.delta.type=input_json_delta` and `partial_json`; `result.usage` | `toolCallId`, `toolName`, `title`, `rawInput`, `Usage` | `content_block_stop` is not a tool completion; completion comes from the result/turn lifecycle |
| Codex app-server | `item/started` / `item/completed` with `params.item.id`, `type`, `status`; command output notifications with `itemId` and `delta`; `thread/tokenUsage/updated` | `toolCallId`, `toolKind`, `status`, `rawInput`, `rawOutput`, `ToolOutputDelta`, `Usage` | An item without a stable id is grouped under the adapter fallback id and remains marked as provider-derived |
| Pi RPC | `message_update.assistantMessageEvent` types `text_delta`, `thinking_delta`, `toolcall_start`, `toolcall_delta`, `toolcall_end`; `tool_execution_*`; `bash_execution_update`; top-level `usage` | text/thought/tool upserts, streamed tool output, status, `Usage` | `toolcall_delta` is buffered until its JSON arguments parse; incomplete JSON is not fabricated as input |
| Antigravity stream-json | `step_update` payload `text_delta`, `step_type`, `tool_name`/`toolInfo`, `tool_input`/`toolInput`, `tool_output`/`toolOutput`, `status`; `result` | text, limited tool update, status, `Usage` where reported | A step with `step_type=unknown` and no tool detail is not rendered as an empty tool card |
| ACP v1 | `agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, mode/config/session updates | ACP-shaped `AgentEvent` | Stable v1 has `toolCallId`, `title`, `kind`, `status`, content/locations/raw input/output and meta; it does not have a separate `name` field, so `toolName` stays absent |

Every mapping has an `Unknown { typeName, raw }` escape hatch. Adding a provider event must
either map it to an existing semantic or preserve the complete raw object; dropping an unknown
frame is a protocol regression. Optional `rawInput`, `rawOutput`, `content`, locations, and
terminal metadata are omitted when unavailable, rather than serialized as `null` patches that
erase information already collected for the same `toolCallId`.

Lifecycle-only frames are the exception to user-visible preservation: a normalizer consumes
known handshake/turn metadata such as Claude `stream_event.message_start`,
`stream_event.content_block_stop`, `stream_event.ping`, and Codex
`thread/started`/`turn/started`, `account/rateLimits/updated`, and `turn/diff/updated` without
creating a conversation block. These transparent events also cannot split a neighboring tool
group. An actually unknown frame is still preserved as `other`; adjacent unknown frames are
projected into one collapsed `ProviderEventGroup` so diagnostics remain available without
becoming a full-size assistant message. Tool calls remain separate keyed rows and are only
merged by the existing call-group renderer. When an unknown event or thought is adjacent to one
or more tool calls, the renderer places the ordered event/tool/event sequence inside that same
call group. Normal assistant `TextDelta`, final reply text, images, permissions, and plan updates
are message/control boundaries and remain outside the call group.

### Migration compatibility

The current `RuntimeAdapter` already centralizes `prompt`, `cancel`, slot discard/reconnect,
teardown, idle reclamation, mode, and config operations. Resume is currently represented by a
persisted provider handle passed into `prompt`; it is not yet a standalone trait method. New
normalizers should be added behind this existing lifecycle seam rather than introducing a
second runtime manager. The frontend `AgentEventEnvelope` is likewise introduced as a
compatibility conversion around the existing `AcpEvent` stream before the Tauri payload is
versioned end-to-end.

The application lifecycle is persisted independently of the provider slot in `agent_lifecycle`.
It is keyed by app session, project role, and runtime, and uses the same state vocabulary for
ACP, native Codex, Pi, Claude, and agy. A project role is the only runtime-owned role entity;
global role data is a template/config source and never owns an app-session lifecycle.

Project deletion is an app-level unlink: it marks the project deleted and filters it from active
lists. It does not delete provider sessions or their Jockey bindings, so reopening the same
project path can recover its local configuration and history.

Provider session administration is exposed for runtimes whose protocols support it (verified against the shipped CLIs): native Codex supports list (`thread/list`), import, fork (`thread/fork`), and rewind (`thread/rollback`) through the `provider_session_cmd` commands; native Pi supports import only (resume via `--session <id>`), since its RPC layer has no list/fork/rollback methods.

The Claude ACP bridge is resolved in strict order: a managed binary under the app data dir, a PATH-installed binary, a controlled `npm install --prefix <app_data_dir>/adapters` of the pinned package (recorded in a `bridge-install.json` manifest), and finally a package-runner (`pnpm dlx` / `npx -y`) fallback that is only enabled for development builds or when `JOCKEY_ALLOW_PACKAGE_RUNNER=1` is set explicitly for diagnostics.

Native Codex MCP is intentionally not mapped: the Codex app-server protocol (verified against codex 0.150.1's v2 schema) exposes no per-thread MCP channel — servers are `~/.codex/config.toml`-level only. Roles with MCP bindings running on native Codex surface a visible warning instead of silently dropping the bindings, and the profile capability stays `false`.
