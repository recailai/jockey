# Mobile control center and transport review

Reviewed 2026-10-23. This is a source review and local regression result, not a claim that every native agent was exercised on a physical phone.

## Product boundary

The phone opens a decision center. Pending approvals, structured questions and agent errors precede running and recent sessions. Approval controls are directly on each session card; no dialog is required to allow or reject. The batch action includes the request count and grants only explicit one-time permissions, never permanent/remembered options. It snapshots the displayed requests, reports success/failure counts and refreshes authoritative pending state. Opening a title reveals the existing chat for context. Complex question forms still open explicitly from an Answer questions button. Long press and an accessible message button open quick input; busy-session input still goes through the engine queue. Projects, imports and session creation remain available behind the session browser.

Partner chat uses the shared `MessageWindow` with `presentation="responses"`. Its presentation projection filters `messageType`/`purpose`, supports legacy untyped messages, shows the authoritative settled `text` rather than tool/commentary segments, and hides live checkpoints, thoughts, plans and telemetry. User input, final responses, images, failures, permissions, questions and queue controls remain available. The projection never mutates the durable transcript or desktop full view.

`components/ControlCenter.tsx` and `hooks/useControlCenter.ts` are shared by desktop and companion. Both use the existing permission/question components, engine API and semantic UI primitives. The desktop has a Control center entry. The phone shell supplies navigation and HTTP transport rather than a second copy of the decision UI.

## Confirmed regressions addressed

| Problem | Consequence | Change |
| --- | --- | --- |
| `useMessageSend` called secure-context-only `crypto.randomUUID()` outside error handling | Sending over LAN/Tailscale HTTP failed before any request; localhost testing missed it | Shared client ID generator uses cryptographic `getRandomValues` when `randomUUID` is absent |
| Message copying assumed `navigator.clipboard` existed | Copy failed on ordinary HTTP | Shared copy helper with synchronous clipboard fallback |
| Companion cleared input before submission with no recovery | A failed send discarded the draft | Restore the draft on failure if the user has not entered newer input |
| Phone dialog CSS was scoped beneath `.companion-root` | Kobalte portals outside the root missed phone overrides | Scope to the companion document; constrain dialog height, input sizing and touch targets |
| Layout used viewport height without the software keyboard's visible viewport | Composer and actions could be obscured while typing | Track visual viewport height in the companion host |
| Approval was removed before the response succeeded | Failed decisions looked accepted | Await success, prevent repeated submission and retain errors/request on failure |
| Structured answer swallowed failure and removed the request in `finally` | Answers disappeared despite failed delivery | Preserve request and answers on failure; add a defer action in the center |
| Role selection submitted twice and detail resync only merged messages | Displayed route could diverge from the service | Single submission; refresh authoritative role/runtime metadata |
| Image paste callback was a no-op | Paste was intercepted and silently dropped | Send image nodes through the existing attachment contract with capability checks |
| SSE stayed open after logout or its last listener unmounted | Stale connections outlived consumers | Close the stream at both lifecycle boundaries |
| Desktop summary command never filled `running` | Shared overview could show a busy agent as idle | Fill running state from the engine |
| Summary omitted lifecycle errors | Phone overview could hide an actionable failure | Add the latest lifecycle error to the shared summary, clear it after reconnect |
| Initial tool event omitted `parentId` and `diff` while updates retained them | Nested tools and patches were incomplete until another update arrived | Retain those fields on initial tool calls too |

## Event and interaction coverage

| Surface | Shared implementation | Current mobile presentation |
| --- | --- | --- |
| Approval requests | `PermissionModal`, `assistantApi.respondPermission` | Inline allow/reject on the card, batch allow-once; inspect chat context when needed |
| Structured questions: single/multiple/free/secret answers | `UserInputModal`, `respondUserInput` | Open from center; defer without answering; retain failed answers |
| Text/thought/tool/output/patch/nested tools | `acpEventBridge`, `AgentBlockRenderer`, `ToolCallGroup` | Existing chat detail; overview only summarizes state |
| Plan/usage/notice/context compaction | Shared session reducer and message UI | Chat detail; not a separate phone implementation |
| Unknown provider blocks | `ProviderEventGroup` fallback | Inspectable in chat; unknown types are not advertised as actionable decisions |
| Agent errors | Lifecycle summary plus existing error banner | Actionable overview row, then chat for recovery |
| New session/send/stop/queue | Engine commands and shared sending hook | Sessions browser/chat plus center quick send/stop |
| Reconnect, resync, pending recovery | Shared events plus HTTP/SSE adapter | Refresh on lifecycle changes, visibility and a 15-second recovery poll |
| Model/mode/CLI command discovery and completion | Desktop API exists; HTTP adapter currently returns empty discovery results | Incomplete; mobile composer no longer advertises a completion menu |
| Native terminal, workspace files, editor and OS integration | Desktop host | Not ported to phone; should remain capability-gated rather than inert controls |

## Architecture findings and next boundaries

1. The engine is the correct shared service boundary. Desktop IPC and HTTP already submit through the same engine. Switching from Tauri to Electron would not fix missing HTTP mappings, browser capability assumptions or lost decision acknowledgements.
2. `Transport.call<T>(command, args)` promises more uniformity than the HTTP adapter supplies. Its handwritten switch reconstructs some results and silently ignores some session update fields; creation also drops desktop-only arguments. Do not blindly reuse every desktop hook until these contract gaps are explicit. Add a typed supported-operation/capability contract and contract tests per operation rather than exposing arbitrary IPC over HTTP.
3. The gateway still holds a Tauri `AppHandle`; reconnect/reset call desktop command handlers. Domain operations should move behind an engine/runtime service, with native hosting, event emission and folder/open actions supplied by the host. Keep authentication and static hosting at the gateway boundary.
4. Chat orchestration still exists in both `App.tsx` and `CompanionApp.tsx`, despite shared reducers and components. The new center does not add another orchestration copy. A later extraction should put session loading, route selection and draft management into one application controller, leaving each shell responsible for navigation/layout.
5. Mobile completeness does not mean duplicating all desktop features. The decision contract must cover pending requests, acknowledgements, expiry, multi-client resolution, error recovery and context links. Model/editor/terminal configuration can stay on desktop, but its absence must be explicit.

## Validation

- TypeScript strict check and desktop/companion production builds.
- Rust library suite: 201 passed, 2 environment-dependent tests ignored.
- Node regressions: HTTP-compatible UUID generation, engine submission payload mapping, unauthorized/unmounted SSE cleanup, response-only projection and batch permission option selection.
- Browser fixture disables `randomUUID` and async clipboard on loopback to reproduce ordinary HTTP capabilities without exposing the dev server on the LAN.
- 390px iframe fixture for phone layout, inline approvals and long-press quick message. Browser interaction checks cover new session → send → streamed result, quick send, batch allow-once with result counts, response-only legacy chat, and failed approval retaining the request.

The gateway embeds `dist-companion` when the Rust app is compiled. Refreshing a phone cannot update an already-running binary; rebuild and restart the desktop service to serve these changes. Physical-phone keyboard/safe-area behavior and live CLI approvals still require device validation.

## Follow-up: production and runtime audit

The installed phone page revealed an additional production-only cause: Tailwind's companion build root excluded shared `src/components` sources. The compiled CSS lacked `.items-end`, `.justify-end` and other chat utilities, although the development fixture looked correct. `src/index.css` now explicitly includes the shared source tree; `tests/companion-bundle.test.mjs` checks the actual compiled CSS.

The GUI inherited an open-file soft limit of 256, causing Claude to exit before a result. Startup now raises only Jockey's soft limit to 8192 (bounded by the existing hard limit); children inherit it. No system-wide launchd settings change. A subprocess test starts at 256 and verifies both the raised limit and shell-child inheritance. Historical-session startup prewarming and the periodic all-open-session configuration refresh were removed. Installed CLI discovery and on-demand configuration discovery remain; merely listing historical sessions must not create provider conversations.

Additional corrections to the control-center paths:

- HTTP session creation preserves the chosen runtime, profile and client ID. Session updates no longer silently discard title/runtime/profile/cwd; role-only changes still use engine persona routing.
- Machine-wide imports preserve actual transcripts and newly created projects in the same result shape as desktop IPC. Opening chat loads complete history instead of silently truncating at 200 messages.
- The phone agent/persona picker supports available agents, waits for acknowledgement, reports failure and blocks changes during an active turn.
- Drafts follow their session; a failed send cannot restore text into a different conversation. Reopening chat refreshes its persisted history, and an idle authoritative snapshot clears a stale live state after a missed finish event.
- Permission responses reject unknown options, duplicate/expired requests and disconnected receivers. Remembered approvals are cached only after delivery. Structured answers also report expired/disconnected requests. Resolution emits a shared pending-update event to desktop and SSE clients.

The retained layering is host UI → typed operation adapter → engine session/turn/queue services → runtime adapters. Agent and model administration still has some native-command coupling; replacing the Tauri host is not necessary to deliver the project/session/decision workflow. Future cross-agent work should submit through the same engine with explicit session/role targets and reuse the decision queue, rather than add another transcript writer or permission implementation.
