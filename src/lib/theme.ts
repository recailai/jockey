export type UiTheme = "dark" | "light";

export const UI_THEME_KEY = "jockey.ui.theme";

export type UiThemeDefinition = {
  key: UiTheme;
  label: string;
  description: string;
  swatch: string;
  accent: string;
  surface: string;
};

export const UI_THEMES: UiThemeDefinition[] = [
  {
    key: "dark",
    label: "Midnight Aura",
    description: "Low-glare dark workspace",
    swatch: "#0B1020",
    accent: "#8B5CF6",
    surface: "#151A2E",
  },
  {
    key: "light",
    label: "Arctic Minimal",
    description: "Clean light workspace",
    swatch: "#F7F9FC",
    accent: "#2563EB",
    surface: "#E9EEF6",
  },
];

export const normalizeUiTheme = (value: string | null | undefined): UiTheme => {
  if (value === "dark" || value === "light") return value as UiTheme;
  return "dark";
};
