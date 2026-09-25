/**
 * The window-focus seam.
 *
 * A window that is not the user's current window stops asserting itself: the
 * accent drains to a neutral, the selection follows it, the active tab loses
 * its raised edge. That is AppKit's inactive appearance generalised, and like
 * the accent and the material it is a fact about the *window* rather than a
 * theme decision — so it arrives as one and CSS decides what it means.
 *
 * The two halves meet at exactly two functions, named as a pair with
 * `lib/system-accent.ts` and `lib/window-material.ts`:
 *
 *   useEffect(() => watchWindowFocus(applyWindowFocus), []);   // App.tsx
 *
 * `applyWindowFocus` is the whole write side: it puts the answer on
 * `<html data-window-focused>` and the inactive-appearance block in `App.css`
 * is its only consumer. Nothing else in the frontend asks.
 *
 * The read side is the window API rather than a platform command, because
 * focus is one of the few window facts Tauri already reports everywhere —
 * `src/components/window-controls.tsx` needs the same reading for the
 * stoplights and caption glyphs, and subscribes through this seam instead of
 * keeping a second copy of the listener.
 *
 * There is still exactly one window in a plain browser (the dev-invoke bridge
 * has one too), and document focus is the same fact there. So the DOM events
 * are not a second seam but the fallback for when the window API is missing
 * *or* unreachable — which is the normal state of browser-side development,
 * where the bridge answers `metadata` but every invoke behind it fails.
 */

import { getAppWindow } from "@/lib/app-window";

/** Mirrors the window's focus onto `<html data-window-focused>` for CSS to
 *  read — the inactive-appearance block in `App.css` is its only consumer. */
export function applyWindowFocus(focused: boolean): void {
  document.documentElement.dataset.windowFocused = String(focused);
}

/** The fallback reader: the page has focus, or it does not. */
function watchDocumentFocus(onChange: (focused: boolean) => void): () => void {
  const sync = () => onChange(document.hasFocus());
  window.addEventListener("focus", sync);
  window.addEventListener("blur", sync);
  onChange(document.hasFocus());
  return () => {
    window.removeEventListener("focus", sync);
    window.removeEventListener("blur", sync);
  };
}

/**
 * Follows the window's focus and hands each reading to `onChange`.
 *
 * Subscribe before pulling, the order `watchSystemAccent` and
 * `watchWindowMaterial` use: `onFocusChanged` only reports changes, so the
 * first reading is one the listener could otherwise miss by mounting after it
 * happened. The pull that follows is a snapshot, not the only source of truth.
 */
export function watchWindowFocus(onChange: (focused: boolean) => void): () => void {
  const appWindow = getAppWindow();
  if (!appWindow) return watchDocumentFocus(onChange);

  let disposed = false;
  let unlisten: (() => void) | null = null;
  let stopFallback: (() => void) | null = null;

  const start = async () => {
    unlisten = await appWindow.onFocusChanged(({ payload }) => {
      if (!disposed) onChange(payload);
    });
    if (disposed) {
      unlisten();
      unlisten = null;
      return;
    }

    onChange(await appWindow.isFocused());
  };

  void (async () => {
    try {
      await start();
    } catch (error) {
      // Decoded at the boundary: the message is what goes to the log (which
      // `forwardConsoleToLogFile` pipes into the backend file), and a thrown
      // non-Error still gets a description instead of `undefined`.
      const reason = error instanceof Error ? error.message : String(error);
      console.error("Unable to follow the window focus; using document focus", reason);
      if (!disposed) stopFallback = watchDocumentFocus(onChange);
    }
  })();

  return () => {
    disposed = true;
    unlisten?.();
    unlisten = null;
    stopFallback?.();
    stopFallback = null;
  };
}
