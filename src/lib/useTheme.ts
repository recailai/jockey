import { createSignal } from "solid-js";
import { type UiTheme, normalizeUiTheme, UI_THEME_KEY } from "./theme";

export function useTheme() {
  const initialTheme = (): UiTheme => {
    try {
      const raw = window.localStorage.getItem(UI_THEME_KEY);
      const theme = normalizeUiTheme(raw);
      document.documentElement.setAttribute("data-theme", theme);
      return theme;
    } catch {
      document.documentElement.setAttribute("data-theme", "light");
      return "light";
    }
  };

  const [uiTheme, setUiTheme] = createSignal<UiTheme>(initialTheme());

  return { uiTheme, setUiTheme };
}
