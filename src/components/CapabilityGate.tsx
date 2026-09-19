import { Show } from "solid-js";
import type { JSX, ParentProps } from "solid-js";
import type { RuntimeCapabilities } from "./types";

type BooleanCapability = Extract<{
  [K in keyof RuntimeCapabilities]: RuntimeCapabilities[K] extends boolean | undefined ? K : never;
}[keyof RuntimeCapabilities], string>;

type CapabilityGateProps = ParentProps<{
  capabilities?: RuntimeCapabilities | Record<string, unknown>;
  capability: BooleanCapability;
  fallback?: JSX.Element;
}>;

export default function CapabilityGate(props: CapabilityGateProps) {
  const supported = () => props.capabilities?.[props.capability] !== false;
  return <Show when={supported()} fallback={props.fallback}>{props.children}</Show>;
}
