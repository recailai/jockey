import { For, Show, createEffect } from "solid-js";
import type { Accessor, Setter } from "solid-js";
import type { AppMentionItem } from "./types";
import { INTERACTIVE_MOTION } from "./types";
import RichInput, { type RichNode, getPlainText } from "./RichInput";
import { Badge, Button, IconButton } from "./ui";

type ChatInputProps = {
  input: Accessor<string>;
  setInput: Setter<string>;
  richNodes: Accessor<RichNode[]>;
  setRichNodes: (nodes: RichNode[]) => void;
  activeRole: Accessor<string>;
  submitting: Accessor<boolean>;
  queuedCount: Accessor<number>;
  onResetRole: () => void;
  isCustomRole: () => boolean;
  onSubmit: (e: SubmitEvent) => void;
  onInputKeyDown: (e: KeyboardEvent) => void;

  refreshInputCompletions: (value: string, caret: number) => void;
  mentionOpen: Accessor<boolean>;
  mentionItems: Accessor<AppMentionItem[]>;
  mentionActiveIndex: Accessor<number>;
  slashOpen: Accessor<boolean>;
  slashItems: Accessor<AppMentionItem[]>;
  slashActiveIndex: Accessor<number>;
  applyMentionCandidate: (item: AppMentionItem) => void;
  applySlashCandidate: (item: AppMentionItem) => void;
  closeMentionMenu: () => void;
  closeSlashMenu: () => void;
  richInputRef: (el: HTMLDivElement) => void;
  mentionCloseTimerRef: { current: number | null };
  mentionDebounceTimerRef: { current: number | null };
  hasImages: Accessor<boolean>;
  onPasteImage: (items: DataTransferItemList, caretNodes: RichNode[]) => void;
  onRemoveImage: (index: number) => void;
};

function mentionKindColor(kind: string): string {
  if (kind === "role") return "is-role";
  if (kind === "dir") return "is-dir";
  if (kind === "file") return "is-file";
  if (kind === "command") return "is-command";
  if (kind === "skill") return "is-skill";
  return "is-default";
}

export default function ChatInput(props: ChatInputProps) {
  createEffect(() => {
    const idx = props.slashActiveIndex();
    const container = slashListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  createEffect(() => {
    const idx = props.mentionActiveIndex();
    const container = mentionListEl;
    if (!container) return;
    const item = container.children[idx] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  });

  let slashListEl: HTMLDivElement | undefined;
  let mentionListEl: HTMLDivElement | undefined;

  const hasContent = () => !!(getPlainText(props.richNodes()).trim() || props.hasImages());

  const handlePaste = (e: ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    const imageItems = Array.from(items).filter((it) => it.kind === "file" && it.type.startsWith("image/"));
    if (!imageItems.length) return;
    e.preventDefault();
    props.onPasteImage(items, props.richNodes());
  };

  const fakeSubmit = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      const fakeEvent = new Event("submit", { bubbles: true, cancelable: true }) as unknown as SubmitEvent;
      props.onSubmit(fakeEvent);
    }
  };

  return (
    <div class="composer-shell">
      <form
        onSubmit={props.onSubmit}
        class="composer-form"
      >
        <div
          class="composer-card"
          onPaste={handlePaste}
        >
          <div class="composer-row">
            <Button
              variant={props.isCustomRole() ? "default" : "outline"}
              size="sm"
              onClick={() => props.onResetRole()}
              class="composer-role-button motion-safe:transition-colors motion-safe:duration-150"
              title={props.isCustomRole() ? "Click to return to Jockey" : "Jockey mode"}
            >
              {props.activeRole()}
              <svg class="opacity-40" width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
            </Button>

            <RichInput
              ref={props.richInputRef}
              nodes={props.richNodes}
              onRemoveImage={props.onRemoveImage}
              placeholder={props.isCustomRole() ? `Chat with ${props.activeRole()}... (type / for agent commands)` : "Natural language / commands / @role @file:path"}
              class="composer-rich-input rich-input"
              onNodesChange={(nodes) => props.setRichNodes(nodes)}
              onCaretText={(text, caret) => {
                props.setInput(text);
                if (props.mentionDebounceTimerRef.current !== null) window.clearTimeout(props.mentionDebounceTimerRef.current);
                props.mentionDebounceTimerRef.current = window.setTimeout(() => {
                  props.mentionDebounceTimerRef.current = null;
                  props.refreshInputCompletions(text, caret);
                }, 90);
              }}
              onKeyDown={(e) => {
                props.onInputKeyDown(e);
                if (!e.defaultPrevented) fakeSubmit(e);
              }}
              onFocus={() => {
                if (props.mentionCloseTimerRef.current !== null) window.clearTimeout(props.mentionCloseTimerRef.current);
                if (props.mentionDebounceTimerRef.current !== null) {
                  window.clearTimeout(props.mentionDebounceTimerRef.current);
                  props.mentionDebounceTimerRef.current = null;
                }
                const text = getPlainText(props.richNodes());
                props.refreshInputCompletions(text, text.length);
              }}
              onBlur={() => {
                if (props.mentionCloseTimerRef.current !== null) window.clearTimeout(props.mentionCloseTimerRef.current);
                props.mentionCloseTimerRef.current = window.setTimeout(() => {
                  props.closeMentionMenu();
                  props.closeSlashMenu();
                }, 120);
              }}
              onClick={() => {
                const text = getPlainText(props.richNodes());
                props.refreshInputCompletions(text, text.length);
              }}
            />

            <IconButton
              type="submit"
              variant={hasContent() ? "default" : "ghost"}
              class={`composer-send-button motion-safe:transition-colors motion-safe:duration-150 ${INTERACTIVE_MOTION}`}
              classList={{
                "is-queueing": hasContent() && props.submitting(),
                "is-disabled": !hasContent(),
              }}
              title={props.submitting() ? `Add to queue (${props.queuedCount() + 1})` : "Send"}
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" classList={{ "drop-shadow-[0_1px_2px_rgba(0,0,0,0.3)]": hasContent() }}><line x1="12" y1="19" x2="12" y2="5" /><polyline points="5 12 12 5 19 12" /></svg>
            </IconButton>
            <Show when={props.queuedCount() > 0}>
              <Badge tone="warning" class="composer-queue-badge">
                Q{props.queuedCount()}
              </Badge>
            </Show>
          </div>
        </div>
        <Show when={props.slashOpen() && props.slashItems().length > 0}>
          <div ref={(el) => { slashListEl = el; }} class="completion-menu">
            <For each={props.slashItems()}>
              {(item, i) => (
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    props.applySlashCandidate(item);
                  }}
                  class="completion-row"
                  classList={{ "is-active": i() === props.slashActiveIndex() }}
                >
                  <span class="mention-kind-badge">cmd</span>
                  <span class="truncate font-mono text-xs">{item.value}</span>
                  <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={props.mentionOpen() && props.mentionItems().length > 0}>
          <div ref={(el) => { mentionListEl = el; }} class="completion-menu">
            <For each={props.mentionItems()}>
              {(item, i) => (
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    props.applyMentionCandidate(item);
                  }}
                  class="completion-row"
                  classList={{ "is-active": i() === props.mentionActiveIndex() }}
                >
                  <span class={`mention-kind-badge ${mentionKindColor(item.kind)}`}>
                    {item.kind}
                  </span>
                  <span class="truncate font-mono text-xs">{item.value}</span>
                  <span class="ml-auto truncate text-[10px] opacity-70">{item.detail}</span>
                </button>
              )}
            </For>
          </div>
        </Show>
      </form>
    </div>
  );
}
