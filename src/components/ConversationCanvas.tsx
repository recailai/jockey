import { Show, Suspense, createEffect, createMemo, lazy, onCleanup } from "solid-js";
import type { Accessor, JSX } from "solid-js";
import type { AppSession } from "./types";
import { hasConversationContent, CONVERSATION_HERO_TITLE, CONVERSATION_HERO_SUBTITLE } from "../lib/conversationHelpers";
import { loadUiPrefs } from "../lib/uiPrefs";
import { useResize } from "../lib/useResize";
import { closeAllPreviewTabs, closeOtherPreviewTabs, closePreviewTab, setActivePreviewTab } from "../lib/previewTabs";
const PreviewArea = lazy(() => import("./PreviewArea"));

type MutateSession = (id: string, recipe: (s: AppSession) => void) => void;

type ConversationCanvasProps = {
  activeSession: Accessor<AppSession | null>;
  activeSessionId: Accessor<string | null>;
  mutateSession: MutateSession;
  editorRatio: Accessor<number>;
  splitContainerHeight: Accessor<number>;
  splitContainerEl: Accessor<HTMLDivElement | null>;
  setSplitContainerEl: (el: HTMLDivElement | null) => void;
  setSplitContainerHeight: (height: number) => void;
  editorResize: ReturnType<typeof useResize>;
  insertMentionAtCaret: (path: string) => void;
  messages: JSX.Element;
  composer: JSX.Element;
};

export default function ConversationCanvas(props: ConversationCanvasProps) {
  let bodyEl: HTMLDivElement | undefined;

  createEffect(() => {
    const el = bodyEl;
    if (!el) return;
    props.setSplitContainerEl(el);
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) props.setSplitContainerHeight(e.contentRect.height);
    });
    ro.observe(el);
    props.setSplitContainerHeight(el.clientHeight);
    onCleanup(() => ro.disconnect());
  });

  const isEmpty = createMemo(() => !hasConversationContent(props.activeSession()));
  const showPreview = createMemo(
    () => (props.activeSession()?.previewTabs.length ?? 0) > 0 && loadUiPrefs().openDiffsInMain,
  );

  return (
    <div
      class="conversation-canvas"
      classList={{ "is-empty": isEmpty() }}
    >
      <div class="conversation-canvas-body" ref={bodyEl}>
        <Show when={showPreview()}>
          <div
            class="preview-shell shrink-0 overflow-hidden"
            style={{ height: `${Math.round(props.editorRatio() * props.splitContainerHeight())}px` }}
          >
            <Suspense fallback={<div class="flex-1 theme-bg" />}>
              <PreviewArea
                session={props.activeSession}
                appSessionId={() => props.activeSession()?.id}
                onCloseTab={(tabId) => {
                  const sid = props.activeSessionId();
                  if (sid) closePreviewTab(props.mutateSession, sid, tabId);
                }}
                onCloseOthers={(tabId) => {
                  const sid = props.activeSessionId();
                  if (sid) closeOtherPreviewTabs(props.mutateSession, sid, tabId);
                }}
                onCloseAll={() => {
                  const sid = props.activeSessionId();
                  if (sid) closeAllPreviewTabs(props.mutateSession, sid);
                }}
                onActivateTab={(tabId) => {
                  const sid = props.activeSessionId();
                  if (sid) setActivePreviewTab(props.mutateSession, sid, tabId);
                }}
                onAddMention={props.insertMentionAtCaret}
              />
            </Suspense>
          </div>
          <div
            class="resizer-y"
            onMouseDown={props.editorResize.beginResize}
            title="Drag to resize"
          />
          <Show when={props.editorResize.previewPx() !== null}>
            <div
              class="resize-guide-y"
              style={{
                top: `${(props.splitContainerEl()?.getBoundingClientRect().top ?? 0) + (props.editorResize.previewPx() ?? 0)}px`,
              }}
            />
          </Show>
        </Show>

        <Show when={isEmpty()} fallback={
          <div class="conversation-stream">
            {props.messages}
          </div>
        }>
          <div class="conversation-hero">
            <div class="conversation-hero-copy">
              <h1 class="conversation-hero-title">{CONVERSATION_HERO_TITLE}</h1>
              <p class="conversation-hero-subtitle">{CONVERSATION_HERO_SUBTITLE}</p>
              <Show when={props.activeSession()?.cwd}>
                {(cwd) => <p class="conversation-hero-cwd">{cwd()}</p>}
              </Show>
            </div>
            <div class="conversation-hero-composer">
              {props.composer}
            </div>
          </div>
        </Show>
      </div>

      <Show when={!isEmpty()}>
        <div class="conversation-composer-anchor">
          {props.composer}
        </div>
      </Show>
    </div>
  );
}
