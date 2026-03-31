export type UiTheme = "dark" | "light";

export const UI_THEME_KEY = "jockeyui.ui.theme";

export const UI_THEMES: Array<{ key: UiTheme; label: string; swatch: string }> = [
  { key: "dark", label: "Midnight Aura", swatch: "#08081e" },
  { key: "light", label: "Arctic Minimal", swatch: "#F8FAFC" },
];

export const normalizeUiTheme = (value: string | null | undefined): UiTheme => {
  if (value === "dark" || value === "light") return value as UiTheme;
  return "dark";
};
