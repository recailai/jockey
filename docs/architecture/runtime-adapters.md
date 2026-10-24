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

Provider listing, fork and rewind previously implemented through native Codex commands are not advertised. Their command boundaries return an explicit unsupported result until an ACP extension is negotiated. Historical CLI import remains available.

## Validation

Unit tests cover runtime routing, pinned-version detection, data migration with an existing ACP slot, transcript/handle preservation, suppression of setup replay, permission acknowledgements and cancellation isolation. Shared HTTP contract and production CSS checks cover the companion boundary. Real adapter smoke checks exercise standard initialize/new/prompt operations without tools or file modifications; authentication-dependent failures must be reported separately from protocol support.
