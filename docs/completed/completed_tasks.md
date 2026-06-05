# Jockey Completed Milestones & Tasks

This document keeps a record of all implemented features, completed tasks, and landed milestones across the Jockey desktop orchestrator.

## Landed Milestones & Features

### UX & Interface Polish
- **Zed-Style Inline Approvals**: Replaced the floating permission modal with inline approval buttons inside the tool-call card.
- **Redesigned Plan Block**: Implemented numbered rows, in-progress border highlights, completed strikethrough text, and priority badges (`high`/`medium`).
- **ActivityBar & Separator Polish**: Added unified `--ui-separator` tokens, structural divider shadows, and upgraded resize handles (`.resizer-x`/`.resizer-y`) with wider hitboxes and hover animations.
- **Preview Tabs**: Embedded SegmentedControl for tab selections, wide file/diff content previews in the main layout.

### Media & Attachment Input
- **Image Attachments**: Integrated file picker support (PNG/JPG/GIF/WebP/SVG), clipboard paste, drag-and-drop, and preview thumbnail strips.
- **Voice Input**: Integrated microphone recording using Web Speech API; voice transcripts are automatically appended to the input composer.
- **Backend Flow**: Implemented `ImageAttachment` types flowing through `AssistantChatInput` -> `WorkerMsg::Execute` -> `build_prompt_blocks`, and serialized to `ContentBlock::Image` ACP blocks.

### ACP Protocol Alignment & Architecture
- **Full ACP Spec Implementation**: Landed all 11 event variants in the event parser and aligned stream protocols.
- **Session Ownership & Terminals**: Added `meta.terminal_output` advertisement, async terminal stdout/stderr drains via `spawn_local` background loops, and graceful reap on client dropdown/reset.
- **State Writeback**: Implemented automatic state writeback for `CurrentModeUpdate` and `ConfigOptionUpdate` on the active `LiveConnection` without requiring worker round-trips.
- **State & Connection Pool**: Integrated connection pool keyed by app session ID to prevent cross-talk, and two-phase prewarm strategy.
- **Error Propagation**: Typed `AcpErrorCode` and `SessionError` structures threaded to frontend banners.
- **Cold-Start Dedup**: Deduplicated concurrent session cold-starts by tracking `PENDING_COLD_STARTS` in the connection pool.

### Role CRUD & Configuration
- **Redesigned Role Models**: Added model, mode, MCP server list, and auto-approve fields.
- **Extended DB Schema & Migrations**: Implemented migrations to support the new database models.
- **Role Editing**: Created frontend role creation forms, copy/edit sub-commands, and CRUD interfaces.

---

## Technical Debt & Performance
- **Thought Deltas**: Routed thought deltas directly to the UI thinking indicator instead of streaming into the main message body.
- **Monolithic Split**: Modularized both Rust and TypeScript codebases (split backend to `acp/*.rs`, `session/*.rs`, `worker/*.rs`, and cleaned up the UI shell structure).
- **Streaming Buffers**: Implemented direct emit and RAF batching on the frontend.
