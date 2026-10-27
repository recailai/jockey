import { Show, createMemo } from "solid-js";
import type { Accessor, JSX } from "solid-js";
import type { AppSession } from "./types";
import { hasConversationContent, CONVERSATION_HERO_TITLE } from "../lib/conversationHelpers";

type Props = {
  activeSession: Accessor<AppSession | null>;
  messages: JSX.Element;
  composer: JSX.Element;
  preview?: JSX.Element;
  bodyRef?: (element: HTMLDivElement) => void;
};

export default function ConversationLayout(props: Props) {
  const empty = createMemo(() => !hasConversationContent(props.activeSession()));
  return (
    <div class="conversation-canvas" classList={{ "is-empty": empty() }}>
      <div class="conversation-canvas-body" ref={props.bodyRef}>
        {props.preview}
        <Show when={empty()} fallback={<div class="conversation-stream">{props.messages}</div>}>
          <div class="conversation-hero">
            <div class="conversation-hero-copy">
              <h1 class="conversation-hero-title">{CONVERSATION_HERO_TITLE}</h1>
              <Show when={props.activeSession()?.cwd}>
                {(cwd) => <p class="conversation-hero-cwd">{cwd()}</p>}
              </Show>
            </div>
            <div class="conversation-hero-composer">{props.composer}</div>
          </div>
        </Show>
      </div>
      <Show when={!empty()}>
        <div class="conversation-composer-anchor">{props.composer}</div>
      </Show>
    </div>
  );
}
