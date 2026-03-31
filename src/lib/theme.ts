export type UiTheme = "dark" | "light";

export const UI_THEME_KEY = "unionai.ui.theme";

export const UI_THEMES: Array<{ key: UiTheme; label: string; swatch: string }> = [
  { key: "dark", label: "Midnight", swatch: "#18181b" },
  { key: "light", label: "Paper", swatch: "#e2e8f0" },
];

export const normalizeUiTheme = (value: string | null | undefined): UiTheme => {
  if (value === "dark" || value === "light") return value;
  return "dark";
};
