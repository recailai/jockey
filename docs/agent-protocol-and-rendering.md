# Agent protocol and rendering contract

> Status: active internal contract. Provider-specific wire details may change, but the
> envelope, block semantics, capability meaning, and queue invariants in this document are
> compatibility boundaries.

Jockey supports several agent processes, but the UI must not depend on any one provider's wire format. Each adapter translates its provider events into the ACP-shaped event envelope; the frontend then translates that envelope into one ordered block stream.

## Boundary model

```text
provider protocol
  -> RuntimeAdapter lifecycle
  -> ProviderNormalizer semantic conversion
  -> AgentEventEnvelope
  -> Tauri event bridge
  -> session-scoped reducer/update seam
  -> ordered AgentBlock/AppSegment
  -> Solid renderer
```

The provider-specific layer owns field names, process lifecycle, resume identifiers, cancellation, and capability checks. The canonical layer owns only semantics:

- `text`: answer text
- `thought`: reasoning/status text intended for the collapsible thought view
- `tool`: a tool invocation and its later updates
- `image`: an image that is safe to render as an attachment
- `error`: a structured failure surface
- `other`: an explicitly preserved event for a semantic not known by the current UI

Every block may carry `roleName`. A tool call carries the same owner as well, so a multi-agent turn can be partitioned without parsing the final answer string. Block order is authoritative. Adjacent text or thought chunks may be merged only when they belong to the same role and have the same kind. `roleName` is metadata; it must not be encoded into visible text as `[role]` and later parsed back out.

The legacy `thoughtText`, `toolCalls`, and independent image fields remain only as compatibility or indexed views. New streaming code writes the ordered stream first; renderers must not display the same semantic twice from a legacy field and a block.

## Provider mapping

| Runtime | Input mapping | Output mapping | Image behavior |
| --- | --- | --- | --- |
| Claude ACP | ACP content blocks | ACP updates | Native image block |
| Claude stream-json | Anthropic user content blocks | stream events and tool events | Native image block |
| Codex app-server | `localImage` input items | app-server notifications | Materialized local image path |
| Pi RPC | `message` plus `images` | RPC notifications | Native image payload |
| agy stream-json | `user.message.content` string | `step_update` / `result` | Materialized local image references |

The important wire fields are intentionally kept here as a quick reference:

```jsonc
// Claude stream-json input
{"type":"user","message":{"role":"user","content":[
  {"type":"text","text":"inspect this"},
  {"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}
]}}
```

```jsonc
// Codex app-server turn/start input item
{"type":"localImage","path":"/tmp/jockey-codex-images/<id>.png"}
```

```jsonc
// Pi RPC prompt input
{"message":"inspect this","images":[
  {"type":"image","data":"...","mimeType":"image/png"}
]}
```

```jsonc
// agy stream-json input (the public headless contract is text-only)
{"event":"user","message":{"content":"inspect /tmp/jockey-agy-images/<id>.png"}}
```

Output differences are normalized before they reach the UI: Claude and ACP emit text/thought/tool notifications, Codex emits `item/*` and reasoning notifications, Pi emits RPC events, and agy emits `step_update` records with `text_delta` and `tool_info`. None of those provider field names should leak into `AppSegment`.

## Canonical event envelope

The backend event is serialized with camelCase fields and a `kind` discriminator. The useful shared shapes are:

```jsonc
{"kind":"textDelta","text":"partial answer"}
{"kind":"thoughtDelta","text":"partial reasoning"}
{"kind":"toolCall","toolCallId":"call-1","toolName":"bash","title":"Run command","toolKind":"shell","status":"pending","rawInput":{}}
{"kind":"toolCallUpdate","toolCallId":"call-1","status":"completed","rawOutput":{}}
{"kind":"toolOutputDelta","toolCallId":"call-1","delta":"stdout chunk"}
{"kind":"usage","inputTokens":1200,"outputTokens":80,"totalTokens":1280}
```

The transport envelope is versioned and adds producer/session ordering metadata:

```jsonc
{
  "schemaVersion": 1,
  "sessionId": "app-session-1",
  "turnId": "run:42",
  "roleName": "reviewer",
  "runtimeKey": "codex-cli",
  "seq": 17,
  "event": { "kind": "toolOutputDelta", "toolCallId": "call-1", "delta": "stdout" }
}
```

`roleName` is the producer identity; it is not inferred from text. `turnId` and `seq` are used together with the client `runToken` to reject late, duplicated, or out-of-order frames. During compatibility migration, a missing backend `turnId` may be derived from the active run token; this fallback must not become a provider-specific rule.

The current Tauri bridge accepts both the legacy payload shape (`role`, `runtimeKind`,
`appSessionId`, `event`, `seq`) and the versioned shape above. The backend now emits
`schemaVersion`, `turnId`, and the canonical identity fields for all adapter-owned stream
events; `agentEvent.ts` remains as the compatibility conversion for old recordings and
third-party test fixtures. The separate `acp/delta` text batching event carries the same
`schemaVersion`/`turnId` metadata and is treated as a legacy transport for `TextDelta`, not as
a second semantic model.

Tool updates are joined by `toolCallId`, and nested work uses `parentId`. Unknown provider fields stay in `rawInput`, `rawOutput`, `content`, `diff`, or `terminalMeta`; an unknown event kind becomes `other` with its original payload. Consecutive `other` events are rendered as one collapsed provider-event group, while tool calls remain in the existing call-group path. Known control-plane metadata is transparent and cannot split a neighboring tool group. This follows DSH's projection boundary: transport lifecycle is consumed before rendering, and the fallback is diagnostic rather than a second chat-message model.

`toolName` is optional and means the provider's stable tool identifier (`bash`, `Read`,
`run_command`). `title` remains the human-readable label. ACP v1 does not define a separate
tool name field, so its adapter leaves `toolName` absent and uses `kind`, `title`, and the raw
payload for display classification. Claude stream-json, Codex app-server, Pi RPC, and agy
populate `toolName` when their wire protocols provide it. An omitted optional field is not the
same as a JSON `null`: updates must not replace an already-known input or output with null.

The frontend turns those events into the following durable model:

```text
AppSession
├── messages: AppMessage[]
│   └── segments: AppSegment[]
│       ├── { kind: "text", text, roleName? }
│       ├── { kind: "thought", text, roleName? }
│       ├── { kind: "tool", tc, roleName? }
│       └── { kind: "other", type, payload, roleName? }
├── streamSegments: AppSegment[]     live ordered turn
├── toolCalls: Record<toolCallId, AppToolCall>  indexed update cache
├── queuedItems: QueuedItem[]        durable local projection
└── usage / notices / permissions / plan / terminals
```

`streamSegments` is the source of display order. `toolCalls` is an indexed update cache, not a second rendering order. The same `SegmentList`/`StreamSegmentList` expressions render historical and live blocks; `AgentStreamList` uses contiguous role runs for multi-agent cards so interleaved roles remain in the original event order. A single-role stream does not receive a second nested role header.

## SolidJS component contract

The frontend is SolidJS, so the SAR React implementation is a behavioral reference rather than a component source. Components must preserve fine-grained updates instead of recreating a whole session tree for each delta.

| Concern | Jockey rule |
| --- | --- |
| Event ingestion | `acpEventBus` validates session, turn, sequence, and run ownership, then delegates semantics. It does not contain provider branches. |
| State update | Use `setSessions`/`produce` on the smallest session field. Do not replace all sessions for one text or tool delta. |
| Block rendering | `AgentBlockRenderer` is the common block boundary. Message segment lists must use stable block identity/order. |
| Capability gating | `CapabilityGate` hides unsupported surfaces; `undefined` means not declared and remains optimistically renderable during migration. |
| Extension safety | Custom block/tool renderers must be registered through a keyed registry and isolated by a Solid `ErrorBoundary`. |
| Streaming | Provider and event layers batch text/output; UI scroll is coalesced with `requestAnimationFrame`. |

Do not port React Context, React hooks, or a global store solely to match SAR. A Solid component may receive an accessor or a narrow callback contract; it should not know the runtime protocol.

The first render path is intentionally bounded: `MESSAGE_RENDER_WINDOW` limits historical rows, while live segments remain fully ordered. Virtualized rendering is a later optimization and requires profiling because tool cards, terminals, and preview interactions have different measurement costs.

## Capability matrix

Capabilities describe what an adapter can actually deliver, not what the underlying model might theoretically support:

| Capability | Claude ACP | Claude native | Codex | Pi | agy |
| --- | ---: | ---: | ---: | ---: | ---: |
| Streaming text | yes | yes | yes | yes | yes |
| Thought/reasoning events | runtime-dependent | runtime-dependent | yes when emitted | runtime-dependent | not guaranteed in headless output |
| Structured tool calls | yes | yes | yes | yes | limited detail |
| Image input | inline ACP block | Anthropic image block | local image path | `images` array | local file reference fallback |
| Permission UI | yes | yes | no | yes | no |
| Resume handle | ACP session id | Claude session id | thread id | Pi session id | conversation id |
| Native MCP mapping | yes | yes | adapter-dependent | adapter-dependent | no |

An adapter must set a capability to `false` when its wire protocol cannot produce the corresponding canonical event. The UI should degrade to a shared fallback component instead of assuming every provider has the same fields.

agy's interactive CLI accepts pasted media, while its documented headless `stream-json` input accepts only text blocks. The agy adapter therefore does not send an unsupported image block that would terminate the stream. It writes validated images to a temporary directory and includes their paths in the prompt, allowing the agent's file/image inspection tools to consume them. If agy adds a stable headless media block, only this adapter should change.

## Multi-agent turns

The backend may run several role targets sequentially even though their events share one app session. The event envelope's `role` identifies the current producer. The frontend preserves that value on every text, thought, and tool segment, then renders each role with the same `AgentStreamList` component. Final `roleReplies` are matched back to those segment groups; no provider-specific response parsing is used.

Adding a new provider requires only its adapter mapping, normalizer, and capability declaration. It must not introduce a second frontend message shape for thoughts, tools, images, or subagents.

## Queue and input delivery

Queue state is item-based, not parallel arrays:

```ts
type QueuedItem = {
  id: string;
  text: string;
  attachments: ImageAttachment[];
  roleName?: string | null;
  delivery: "nextTurn" | "nextStep";
  createdAt: number;
  status: "queued" | "claimed" | "failed";
};
```

Delivery is an explicit lifecycle:

```text
enqueue -> claim(ids) -> prompt -> ack(remove) 
                         \-> restore(ids) on failure
```

The frontend must never implement queue delivery as `list all -> clear inbox -> prompt`. Claiming is conditional on the item still being unclaimed, and only the successful sender may acknowledge it. If an adapter cannot support `nextStep`, the adapter chooses a declared fallback; the UI must preserve the requested and delivered modes when that distinction is user-visible.

## Cancellation and queued sends

`Send now` follows a drain barrier:

1. mark the current run cancelled and stop accepting late events;
2. request provider cancellation;
3. wait until the provider turn releases its per-session execution slot;
4. only then dequeue and send the next message.

This is equivalent to dsh's `cancel` plus `whenIdle`: a wakeup that arrives during abort is latched, but it is not executed before the old turn has converged to idle. Headless runtimes use an explicit active-count drain; ACP already acknowledges after its prompt lock is released.

The adapter strategy is explicit. Native adapters with a concurrent control writer use their
provider's steer operation; transports without one use the safe cancel-and-drain fallback:

| Runtime | Provider-native control semantics | Jockey strategy today |
| --- | --- | --- |
| Pi RPC | `steer` interrupts after the current tool; `follow_up` waits until the turn settles. | `steer`; attachments/role changes fall back to interrupt-and-drain. |
| Codex app-server | `turn/steer` appends input to the active regular turn; `turn/interrupt` ends it as interrupted. | `steer` with `threadId` + `expectedTurnId`; if the active turn has no steerable id, fall back to interrupt-and-drain. |
| Claude ACP | ACP standard exposes `session/prompt` and `session/cancel`, not a provider-neutral steer. | `interruptFollowUp`. |
| Claude stream-json | Bidirectional stream input exists, but this runner serializes one prompt per execution slot. | `interruptFollowUp`. |
| Antigravity stream-json | No stable mid-turn steer contract is exposed by the current headless adapter. | `interruptFollowUp`. |

The UI label describes the declared strategy: a true `steer` adapter may show `Steer now`; the
current fallback shows `Send now` with an interrupt-and-drain tooltip. The ordinary Escape/Stop
action remains a cancellation and continues to show `Cancellation requested.` only for that action.

The invariant is that a cancellation marker may never be consumed by a newly started turn. A drain timeout is surfaced as an error and prevents the frontend from launching the queued turn blindly.

The queue runner is also session-scoped and mutually exclusive. A second `Send now` click while
that session is already claiming or sending a queued item is ignored by the command path and
shown as `Sending…` in the UI; it must not cancel the newly started queued turn. Cancellation
messages, mode notices, and reconnect notices are appended to the originating session rather
than whichever session happens to be active when the async operation completes. Backend cancel
resolution filters lifecycle rows by the requested role, so cancelling one role cannot stop a
co-running sibling role in the same app session.

Legacy workflow events use their serialized `sessionId` when no `appSessionId` is present. They
remain an out-of-band event stream and are buffered/merged separately from canonical assistant
blocks; they must not be mistaken for a provider turn or used to adopt a new `turnId` after a
cancellation.

Queued input is also persisted in `session_inbox_messages`. Its stable id, target role,
delivery mode (`nextTurn` or `nextStep`), text, and attachments are stored before the local
queue indicator is updated. The UI currently exposes the common queued list and `Send now`
action; the delivery field leaves room for providers with a native step-level steer without
leaking that provider's request shape into the UI.

## Role handoff context and Jockey MCP

The recent-role handoff is deterministic application context, not an MCP side effect. A
successful turn is stored in the app-session `recentRoleChats` snapshot with a bounded history:
at most three turns per role and nine turns across the session. Automatic injection is now
opt-in: the default context mode is `none`, so switching roles does not silently enlarge the
next prompt. Callers that explicitly need prompt-level handoff can use `mode: "handoff"` or
`mode: "history"`; the latter can include the current role. This keeps the normal path small
and lets an agent decide when historical context is relevant through MCP.

The injected value is a typed reference block rather than a concatenated transcript:

```jsonc
{
  "type": "roleContext",
  "purpose": "roleHandoff",
  "targetRole": "Reviewer",
  "messageTypes": ["user", "assistant"],
  "turns": [{
    "roleName": "Developer",
    "messageType": "turn",
    "purpose": "roleHandoff",
    "messages": [
      {"messageType": "user", "purpose": "request", "content": "..."},
      {"messageType": "assistant", "purpose": "response", "content": "..."}
    ]
  }]
}
```

The content is explicitly reference data. It must not be treated as a new instruction, and
role names must remain metadata rather than `[role]` text prefixes. Callers may pass the
optional chat context parameters `mode`, `recentTurns`, and `includeCurrentRole`; the backend
clamps the turn count to 1–8.

The built-in local `jockey` MCP server is available to every role by default when the selected
adapter declares MCP support. A role can still disable it through its explicit MCP flag. Native
Codex, Pi, and agy currently do not receive the auto-injected server because their adapters
cannot map per-session MCP; this avoids a misleading “ignored MCP” notice on every turn.

The default advertised Jockey MCP surface is deliberately small: `get_session_context`,
`list_roles`, `list_skills`, `get_skill`, and explicit `invoke_role`. Management methods for
roles, MCP registrations, sessions, shared context, and workflows remain dispatch-compatible
for the desktop application but are not advertised to agents. Workflow persistence exists, but
workflow execution semantics are not stable enough to expose as an agent tool yet.

`get_session_context` is the on-demand history API. It accepts `appSessionId`, a bounded
`limit` (1–50, default 5), and the views `summary`, `roles`, `turns`, and `messages`. Messages
can be filtered by exact `roleName` and semantic `messageTypes` (`user`, `assistant`, `tool`,
`thought`, `event`), paged with `cursor`, and returned with only selected fields through
`include` (`text`, `toolSummary`, `tools`, `toolOutput`, `raw`). `summary` gives a compact
session/role overview plus recent messages and grouped turns; `turns` groups adjacent persisted
messages around user boundaries. Provider stream events that are not persisted as app-session
messages are intentionally not fabricated into history. `get_session_history` and
`get_role_context` remain compatibility aliases, while new adapters and prompts should use
`get_session_context`.

Each `(appSessionId, roleName, runtimeKind)` has the same lifecycle state machine:
`idle -> prewarming -> ready -> running -> stopping -> stopped`, with `error` as a recoverable
state. Transitions are persisted with a revision and last error, so a provider-specific
process slot is an implementation detail rather than the lifecycle model presented to Jockey.

Global role rows are treated as templates when a project role is present. A project role keeps
only its changed fields in `project_role_overrides`; its effective runtime configuration is the
global template merged with those overrides. Project deletion sets `projects.deleted_at` and
leaves sessions, provider handles, role bindings, and project configuration recoverable.

## Adding fields safely

Provider fields belong in the adapter event conversion and may remain in `rawInput`, `rawOutput`, or `terminalMeta` when they have no shared UI meaning. Promote a field into the canonical event only when at least two runtimes can express the same semantic and the renderer has one stable presentation for it.

## Solid performance checklist

- Provider parsing happens once in Rust; the WebView receives compact canonical events.
- Text, thought, tool output, and status updates have separate batching policies; do not put all events through one timer.
- Every scroll request is RAF-coalesced and must respect the user's scrolled-up state.
- Tool output is appended as a delta and capped or truncated at the presentation boundary.
- `null` usage values remain unknown; they must not render as zero.
- A malformed or unsupported block becomes an error/other card, never a page-level failure.
- Add a profiling trace before introducing virtualization, memo layers, or a second state cache.

## Alignment checklist

When adding or changing an agent adapter, update these together:

1. `HeadlessProtocol` or `NativeProtocol` and its input/output parser.
2. `RuntimeCapabilities` and the runtime profile's transport label.
3. Conversion to `AcpEvent`, including `toolCallId`, `parentId`, usage, and session handle.
4. `AgentEvent`/`AgentBlock` only if the semantic is genuinely shared by the UI.
5. The provider row, capability row, and a wire example in this document.
6. Queue delivery behavior, if the runtime has a native steer or resume path.
7. A parser, capability, reducer/replay, or queue test that covers the provider's actual field spelling.

The final acceptance criterion is provider interchangeability at the renderer boundary: changing the runtime may change the adapter and the available capabilities, but it must not change the component expression used for text, thought, tool, usage, or multi-agent output.
