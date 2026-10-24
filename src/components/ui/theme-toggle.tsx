import { Match, Switch } from "solid-js";
import type { Accessor } from "solid-js";
import { Monitor, Moon, Sun } from "lucide-solid";
import { THEME_LABEL, nextThemePreference, type ThemePreference } from "../../lib/theme";
import { ToolbarButton } from "./button";

/** One-click theme cycle (dark → light → system). The icon shows the current choice. */
export function ThemeToggle(props: {
  preference: Accessor<ThemePreference>;
  onCycle: () => void;
  size?: number;
  class?: string;
  shortcut?: string;
}) {
  const title = () => {
    const next = THEME_LABEL[nextThemePreference(props.preference())];
    const hint = props.shortcut ? ` (${props.shortcut})` : "";
    return `Theme: ${THEME_LABEL[props.preference()]} — switch to ${next}${hint}`;
  };
  const size = () => props.size ?? 15;
  return (
    <ToolbarButton class={props.class} title={title()} aria-label={title()} onClick={() => props.onCycle()}>
      <Switch>
        <Match when={props.preference() === "dark"}><Moon size={size()} stroke-width={1.75} /></Match>
        <Match when={props.preference() === "light"}><Sun size={size()} stroke-width={1.75} /></Match>
        <Match when={props.preference() === "system"}><Monitor size={size()} stroke-width={1.75} /></Match>
      </Switch>
    </ToolbarButton>
  );
}
