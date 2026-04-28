import type { JSX, ParentProps } from "solid-js";
import { splitProps } from "solid-js";
import * as KTabs from "@kobalte/core/tabs";

type TabsRootProps = ParentProps<{
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  class?: string;
}>;

export function Tabs(props: TabsRootProps) {
  const [local, rest] = splitProps(props, ["class", "children", "onChange"]);
  return (
    <KTabs.Root {...rest} onChange={local.onChange} class={`jui-tabs ${local.class ?? ""}`}>
      {local.children}
    </KTabs.Root>
  );
}

export function TabsList(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <KTabs.List {...rest} class={`jui-tabs-list ${local.class ?? ""}`}>{local.children}</KTabs.List>;
}

export function TabsTrigger(props: ParentProps<{
  value: string;
  class?: string;
  disabled?: boolean;
  id?: string;
  title?: string;
}>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <KTabs.Trigger {...rest} class={`jui-tabs-trigger ${local.class ?? ""}`}>{local.children}</KTabs.Trigger>;
}

export function TabsContent(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement> & { value: string }>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <KTabs.Content {...rest} class={`jui-tabs-content ${local.class ?? ""}`}>{local.children}</KTabs.Content>;
}

export function SegmentedControl(props: ParentProps<JSX.HTMLAttributes<HTMLDivElement>>) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return <div {...rest} class={`jui-segmented ${local.class ?? ""}`}>{local.children}</div>;
}

export function SegmentButton(props: ParentProps<JSX.ButtonHTMLAttributes<HTMLButtonElement> & { active?: boolean }>) {
  const [local, rest] = splitProps(props, ["class", "children", "active"]);
  return (
    <button
      type="button"
      {...rest}
      class={`jui-segment ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
    >
      {local.children}
    </button>
  );
}
