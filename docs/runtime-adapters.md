# Runtime adapter architecture

Jockey keeps `ACP` as the product-level runtime category. ACP is the common event and permission surface exposed to the Solid UI; it is not a requirement that every CLI implement ACP itself.

Each runtime is resolved into an adapter descriptor with these fields:

- `kind` / `runtimeKey`: stable provider identity used by roles, sessions, and persisted handles.
- `binary` / `args` / `env`: the resolved launch contract after PATH and interactive-shell resolution.
- `launchMethod`: diagnostic metadata such as `native:codex-app-server`, `native:pi-rpc`, `headless-binary`, or `package-runner:pnpm`.
- `transport`: the protocol implementation selected for the runtime.

Current transport mapping:

| Runtime | Transport | Native session handle |
| --- | --- | --- |
| Claude Code | ACP bridge | ACP session id |
| Codex CLI | Codex app-server JSON-RPC | Codex thread id |
| Pi CLI | Pi JSONL RPC | Pi session file |
| Antigravity CLI (`agy`) | headless `stream-json` (persistent when supported) | conversation id |

The protocol runners own process launch, JSONL framing, request correlation, stderr tails, cancellation, model discovery, and conversion of provider events into the existing ACP event envelope. The UI therefore does not need provider-specific event types. Each process slot is keyed by `appSessionId + runtimeKey + roleName`, so independent roles cannot share a provider session accidentally.

Model and mode discovery is runtime data, not a hardcoded model catalog. ACP providers update the runtime cache from session state; Codex refreshes through `model/list`; Pi refreshes through `get_available_models`; an explicit role-config refresh clears the previous runtime snapshot before probing again.

Native sessions reuse one long-lived provider process while the app session is active. agy 1.1+ uses the same strategy with `--input-format stream-json --output-format stream-json`: one stdin/stdout process handles multiple turns and returns one `result` event per turn. Older agy builds fall back to one-shot headless JSON when the stream flags are not advertised. If a process exits, the cwd/launch contract changes, or the session is idle for five minutes, the slot is evicted and the persisted provider handle is used for recovery: Codex via `thread/resume`, Pi via `--session <session-file>`, and agy via `--conversation <id>` when a new process is needed.

Per-tool-use permission for the native Claude provider is bridged through the `--permission-prompt-tool` contract: the runner passes a temp `--mcp-config` pointing at `current_exe --__jockey-permission-bridge`, a minimal stdio MCP server that forwards each request over a loopback socket into the shared Jockey permission UI. Requests block until the user answers, are denied on timeout, and are denied when the bridge disconnects. Roles that explicitly enable auto-approve still run with `--dangerously-skip-permissions`; without it, a session whose bridge is unavailable refuses to start instead of silently bypassing. Native MCP config mapping for agy remains future work; until then, the native runner records unmapped MCP counts in diagnostics instead of silently claiming native MCP support.

The agy stream protocol is the current documented headless integration: an `init` event is followed by `step_update` events and exactly one `result` event for each prompt. Jockey consumes the provider's `response` and `text_delta` fields, forwards tool steps into the ACP event envelope, and preserves the conversation id for the next turn. See the [official agy headless-mode documentation](https://www.agy.dev/docs/cli/headless/) for the CLI contract.

ACP itself is still actively maintained. Jockey now uses the official Rust SDK `2.0.0` with the stable `schema::v1` wire types. The SDK's high-level connection builder is intentionally not used here: the worker is a single-threaded Tokio `LocalSet` and the adapter layer needs one uniform process lifecycle, pending-request map, backpressure point, stderr tail, and shutdown path across ACP, Codex, Pi, and agy. `acp/transport.rs` therefore owns the small JSON-RPC-over-JSONL boundary while the SDK owns protocol types and serialization contracts. The old `unstable_session_model` API was removed; Claude model choices come from ACP `session/config_options`, while Codex and Pi expose their native model catalogs. The transport splits SDK 2.0 batch frames (a JSON array of JSON-RPC messages) into independently dispatched members, times out pending requests after 120 seconds, flushes every waiter on EOF, and logs orphan responses and unknown notifications for diagnosis.

All runtimes are dispatched through the `RuntimeAdapter` trait (`session/adapter_runtime.rs`), which fixes the lifecycle entry points — `probe`, `prompt`, `cancel`, `discard_slot`, `reconnect_slot`, `teardown`, `reclaim_idle`, `diagnostics` — so callers (execute dispatch, session commands, worker sweeps) never branch on transport kind. The enum-backed dispatch keeps per-transport runners where the verified protocol logic lives; a fake adapter implements the same trait for the unit-test matrix.

Provider session administration is exposed for runtimes whose protocols support it (verified against the shipped CLIs): native Codex supports list (`thread/list`), import, fork (`thread/fork`), and rewind (`thread/rollback`) through the `provider_session_cmd` commands; native Pi supports import only (resume via `--session <id>`), since its RPC layer has no list/fork/rollback methods.

The Claude ACP bridge is resolved in strict order: a managed binary under the app data dir, a PATH-installed binary, a controlled `npm install --prefix <app_data_dir>/adapters` of the pinned package (recorded in a `bridge-install.json` manifest), and finally a package-runner (`pnpm dlx` / `npx -y`) fallback that is only enabled for development builds or when `JOCKEY_ALLOW_PACKAGE_RUNNER=1` is set explicitly for diagnostics.

Native Codex MCP is intentionally not mapped: the Codex app-server protocol (verified against codex 0.150.1's v2 schema) exposes no per-thread MCP channel — servers are `~/.codex/config.toml`-level only. Roles with MCP bindings running on native Codex surface a visible warning instead of silently dropping the bindings, and the profile capability stays `false`.
