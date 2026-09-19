import { ErrorBoundary, Match, Switch } from "solid-js";
import type { JSX } from "solid-js";
import type { AgentBlock } from "./types";
import { Alert, Badge } from "./ui";
import { ToolCallGroup } from "./ToolCallGroup";
import { ProviderEventGroup } from "./ProviderEventGroup";

type AgentBlockRendererProps = {
  block: AgentBlock;
  renderText: (text: string) => string;
  cwd?: string | null;
  terminals?: Parameters<typeof ToolCallGroup>[0]["terminals"];
  streaming?: boolean;
};

export function AgentBlockRenderer(props: AgentBlockRendererProps): JSX.Element {
  return (
    <ErrorBoundary fallback={(error) => (
      <Alert tone="danger" title="Block render failed">
        {String(error)}
      </Alert>
    )}>
    <Switch>
      <Match when={props.block.kind === "text"}>
        <div class="md-prose" innerHTML={props.renderText((props.block as Extract<AgentBlock, { kind: "text" }>).text)} />
      </Match>
      <Match when={props.block.kind === "thought"}>
        <details class="agent-thought-block">
          <summary><Badge tone="neutral" variant="subtle">Thought</Badge></summary>
          <div class="md-prose whitespace-pre-wrap">{(props.block as Extract<AgentBlock, { kind: "thought" }>).text}</div>
        </details>
      </Match>
      <Match when={props.block.kind === "tool"}>
        <ToolCallGroup
          tools={[(props.block as Extract<AgentBlock, { kind: "tool" }>).tc]}
          streaming={props.streaming ?? false}
          cwd={props.cwd}
          terminals={props.terminals}
        />
      </Match>
      <Match when={props.block.kind === "error"}>
        <Alert tone="danger" title={(props.block as Extract<AgentBlock, { kind: "error" }>).code}>
          {(props.block as Extract<AgentBlock, { kind: "error" }>).message}
        </Alert>
      </Match>
      <Match when={props.block.kind === "other"}>
        <ProviderEventGroup events={[{
          type: (props.block as Extract<AgentBlock, { kind: "other" }>).type,
          payload: (props.block as Extract<AgentBlock, { kind: "other" }>).payload,
        }]} />
      </Match>
      <Match when={props.block.kind === "image"}>
        <img
          class="max-w-full rounded-lg"
          src={`data:${(props.block as Extract<AgentBlock, { kind: "image" }>).mimeType};base64,${(props.block as Extract<AgentBlock, { kind: "image" }>).data}`}
          alt="Agent attachment"
        />
      </Match>
    </Switch>
    </ErrorBoundary>
  );
}
