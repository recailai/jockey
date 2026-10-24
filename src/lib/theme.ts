/** What is painted. */
export type UiTheme = "dark" | "light";
/** What the user picked; `system` follows the OS appearance live. */
export type ThemePreference = UiTheme | "system";

export const UI_THEME_KEY = "jockey.ui.theme";

export type UiThemeDefinition = {
  key: ThemePreference;
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
  {
    key: "system",
    label: "Match system",
    description: "Follows your OS appearance",
    swatch: "linear-gradient(135deg, #F7F9FC 50%, #0B1020 50%)",
    accent: "#64748B",
    surface: "#94A3B8",
  },
];

export const THEME_LABEL: Record<ThemePreference, string> = {
  dark: "Dark",
  light: "Light",
  system: "System",
};

export const normalizeThemePreference = (value: string | null | undefined): ThemePreference => {
  if (value === "dark" || value === "light" || value === "system") return value;
  return "dark";
};

/** Order of the quick toggle (topbar button, ⌘⇧L, companion header). */
export const nextThemePreference = (pref: ThemePreference): ThemePreference =>
  pref === "dark" ? "light" : pref === "light" ? "system" : "dark";

export const resolveTheme = (pref: ThemePreference, systemDark: boolean): UiTheme =>
  pref === "system" ? (systemDark ? "dark" : "light") : pref;

export function loadThemePreference(): ThemePreference {
  try {
    return normalizeThemePreference(window.localStorage.getItem(UI_THEME_KEY));
  } catch {
    return "dark";
  }
}

/** Paints `theme` and keeps the browser chrome (mobile status bar) in step with the page. */
export function applyTheme(theme: UiTheme, pref: ThemePreference): void {
  const root = document.documentElement;
  root.setAttribute("data-theme", theme);
  root.style.colorScheme = theme;
  try {
    window.localStorage.setItem(UI_THEME_KEY, pref);
  } catch {
    // Storage can be unavailable (private mode); the theme still applies for this page.
  }
  const meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (meta) {
    const bg = getComputedStyle(root).getPropertyValue("--ui-bg").trim();
    if (bg) meta.content = bg;
  }
}
