/**
 * Platform helpers for cross-platform hint text (macOS/Linux/Windows).
 * Keyboard event handlers already accept both Ctrl and ⌘ (metaKey); these
 * helpers only affect how shortcuts are displayed to the user.
 */

/** True on macOS, where the primary shortcut modifier is ⌘ instead of Ctrl. */
export const isMacPlatform =
  typeof navigator !== "undefined" && /Mac(?:intosh|iPhone| OS X)/i.test(navigator.userAgent);

/** True on Windows, where Tauri custom URI schemes map to `http://*.localhost`. */
export const isWindowsPlatform =
  typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

/** Platform-appropriate primary modifier label ("⌘" or "Ctrl"). */
export const MOD_KEY = isMacPlatform ? "⌘" : "Ctrl";
