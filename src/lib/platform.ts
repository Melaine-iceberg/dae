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

/**
 * True when the Electron preload bridge is installed. Electron serves custom
 * schemes under their own names on every platform, unlike WebView2, which
 * maps them to `http://<scheme>.localhost`.
 */
export const isElectronShell =
  typeof window !== "undefined" && "daeBridge" in window;

/** Platform-appropriate primary modifier label ("⌘" or "Ctrl"). */
export const MOD_KEY = isMacPlatform ? "⌘" : "Ctrl";
