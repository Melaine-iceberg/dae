/**
 * The light/dark theme seam.
 *
 * A preference of `light` or `dark` pins the shell; `system` follows the OS.
 * The OS reading is platform work and lives on the Rust side: the
 * `system_theme` module reports it from the freedesktop settings portal on
 * Linux and reports nothing on Windows/macOS, where the WebView's own
 * `prefers-color-scheme` already follows the system appearance.
 *
 * On Linux that indirection is the whole point. This app's WebView is
 * WebKitGTK (GTK 3), which infers `prefers-color-scheme` from the GTK *theme*
 * rather than from the desktop's Light/Dark toggle — so a GTK theme left on a
 * `-dark` variant made "follow system" paint dark under a desktop set to
 * light. The portal publishes the value the desktop's toggle writes, which is
 * what this module now prefers, and `prefers-color-scheme` is kept as the
 * fallback for a platform (or a first read that has not landed) where the
 * backend says nothing.
 */

import { commands, events } from "@/bindings";

export type ThemePreference = "light" | "dark" | "system";

const THEME_STORAGE_KEY = "app.theme";

const darkModeMedia = window.matchMedia("(prefers-color-scheme: dark)");

/**
 * The OS light/dark preference as the backend last reported it, or `null`
 * while the platform publishes none — or has not answered yet.
 *
 * `null` is not "light": it means "no opinion", and `systemPrefersDark` then
 * defers to the media query. Only a real `true`/`false` overrides it.
 */
let reportedSystemDark: boolean | null = null;

/** Whether the OS is drawn dark, preferring the backend's reading. */
function systemPrefersDark(): boolean {
  return reportedSystemDark ?? darkModeMedia.matches;
}

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
  const dark = preference === "dark" || (preference === "system" && systemPrefersDark());
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

/**
 * Follows the OS light/dark preference and calls `onChange` every time it
 * moves.
 *
 * Two sources, in priority order. The backend (`src-tauri/src/system_theme`)
 * knows the desktop's real preference, which is the only correct answer on
 * Linux; the `prefers-color-scheme` media query is the fallback for a platform
 * where the backend reports nothing — and, everywhere, holds the value until
 * the backend's first read lands.
 *
 * The listener is registered *before* the first read on purpose, exactly as
 * `watchSystemAccent` is: the backend watcher starts before the webview mounts,
 * so its opening report can arrive and be dropped. Subscribing first closes
 * that gap, and the pull that follows is then a snapshot rather than the only
 * source of truth. The other order would let a stale snapshot overwrite a
 * change that had already arrived.
 */
export function watchSystemTheme(onChange: () => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;

  // The media query keeps `system` following the OS on the platforms the
  // backend does not cover. It must not override a live portal reading, so it
  // only fires while the backend has nothing to say.
  const onMediaChange = () => {
    if (reportedSystemDark === null) onChange();
  };
  darkModeMedia.addEventListener("change", onMediaChange);

  const start = async () => {
    unlisten = await events.systemThemeChanged.listen(({ payload }) => {
      if (disposed) return;
      reportedSystemDark = payload;
      onChange();
    });
    if (disposed) {
      unlisten();
      unlisten = null;
      return;
    }

    const current = await commands.getSystemTheme();
    if (disposed) return;
    reportedSystemDark = current;
    onChange();
  };

  void start().catch((error: unknown) => {
    console.warn("Unable to follow the system theme", error);
  });

  return () => {
    disposed = true;
    darkModeMedia.removeEventListener("change", onMediaChange);
    unlisten?.();
    unlisten = null;
  };
}
