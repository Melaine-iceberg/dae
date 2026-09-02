export type ThemePreference = "light" | "dark" | "system";

const THEME_STORAGE_KEY = "app.theme";

const darkModeMedia = window.matchMedia("(prefers-color-scheme: dark)");

/** Reads the persisted preference, defaulting to "system". */
export function getStoredThemePreference(): ThemePreference {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

/** Persists the preference and applies it immediately. */
export function setThemePreference(preference: ThemePreference): void {
  localStorage.setItem(THEME_STORAGE_KEY, preference);
  applyThemePreference(preference);
  window.dispatchEvent(new CustomEvent("app-theme-change"));
}

/**
 * Applies a preference (SKILL.md §12): "system" follows the OS, while the
 * explicit modes pin the semantic surface hierarchy regardless of the OS.
 */
export function applyThemePreference(preference: ThemePreference): void {
  const dark = preference === "dark" || (preference === "system" && darkModeMedia.matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
}

/** Re-applies the stored preference; used when the OS scheme changes. */
export function applySystemTheme(): void {
  applyThemePreference(getStoredThemePreference());
  // Notify theme-derived surfaces (e.g. the terminal palette) that the OS
  // scheme flipped, since `applyThemePreference` only toggles the class.
  window.dispatchEvent(new CustomEvent("app-theme-change"));
}

export function watchSystemTheme(onChange: () => void): () => void {
  darkModeMedia.addEventListener("change", onChange);
  return () => darkModeMedia.removeEventListener("change", onChange);
}
