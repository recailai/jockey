import { createEffect, createSignal, onCleanup } from "solid-js";
import {
  type ThemePreference,
  type UiTheme,
  applyTheme,
  loadThemePreference,
  nextThemePreference,
  resolveTheme,
} from "./theme";

/** Shared by the desktop app and the phone companion so both honour the same stored choice. */
export function useTheme() {
  const media = window.matchMedia?.("(prefers-color-scheme: dark)");
  const [preference, setPreference] = createSignal<ThemePreference>(loadThemePreference());
  const [systemDark, setSystemDark] = createSignal(media?.matches ?? true);
  const theme = (): UiTheme => resolveTheme(preference(), systemDark());

  const onSystemChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
  media?.addEventListener("change", onSystemChange);
  onCleanup(() => media?.removeEventListener("change", onSystemChange));

  // Another window (or the companion in a second tab) changed the stored choice.
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key.endsWith(".ui.theme")) setPreference(loadThemePreference());
  };
  window.addEventListener("storage", onStorage);
  onCleanup(() => window.removeEventListener("storage", onStorage));

  applyTheme(theme(), preference());
  createEffect(() => applyTheme(theme(), preference()));

  return {
    preference,
    theme,
    setPreference,
    cycle: () => setPreference(nextThemePreference(preference())),
  };
}
