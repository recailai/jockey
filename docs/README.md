# Documentation index

## architecture/

Current design of the running system.

- [engine-service-layer.md](architecture/engine-service-layer.md): engine that owns turns, queue, and persistence.
- [session-sync.md](architecture/session-sync.md): design for joining a running turn and following terminal-driven provider sessions (not implemented).
- [runtime-adapters.md](architecture/runtime-adapters.md): ACP as the product envelope and the per-CLI adapters.
- [agent-protocol-and-rendering.md](architecture/agent-protocol-and-rendering.md): normalized agent event contract and how it is rendered.
- [ui-design-and-components.md](architecture/ui-design-and-components.md): UI design system and component specification.
- [data-modeling.md](architecture/data-modeling.md): data modeling guidelines.
- [frontend-webview.md](architecture/frontend-webview.md): frontend practices for the Tauri WebView.

## reviews/

- [review-2026-10.md](reviews/review-2026-10.md): October 2026 project review (correctness and performance).

## roadmap/

- [pending.md](roadmap/pending.md): outstanding tasks and planned features.

## reference/

Background material, not a description of Jockey's own code.

- [acp-sdk-notes.md](reference/acp-sdk-notes.md): pitfalls found while using the ACP Rust SDK.
- [acp/README.md](reference/acp/README.md): index of ACP reference client research.
- [acp/zed.md](reference/acp/zed.md): how Zed integrates ACP.
- [acp/aionui.md](reference/acp/aionui.md): how AionUi integrates ACP.
- [acp/best_practices.md](reference/acp/best_practices.md): checklist of patterns worth borrowing.

## archive/

Historical and superseded; kept for context only.

- [agent-orchestrator-design.md](archive/agent-orchestrator-design.md): original orchestrator design.
- [completed-tasks.md](archive/completed-tasks.md): record of completed milestones.

## assets/

- `screenshot.png`, `demo.mp4`: used by the top-level README.
