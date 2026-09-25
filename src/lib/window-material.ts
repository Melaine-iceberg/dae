/**
 * The window-material seam.
 *
 * The parts of this shell that are the *window* — the canvas behind the
 * panels and the nav column — are meant to show whatever the OS composites
 * behind a window: Mica on Windows 11, the sidebar material on macOS. That is
 * the native answer to "what colour is a file manager's chrome", and like the
 * accent it is a value the platform owns rather than one this theme should
 * invent.
 *
 * Deciding whether a backdrop exists is platform work and lives on the Rust
 * side: `src-tauri/src/window_material` asks whether the window may be
 * transparent, applies Mica or vibrancy, and follows the OS setting that turns
 * effects off. This module is the other half — what the answer means to CSS —
 * and the two halves meet at exactly two functions, named as a pair with
 * `lib/system-accent.ts`:
 *
 *   useEffect(() => watchWindowMaterial(applyWindowMaterial), []);   // App.tsx
 *
 * `applyWindowMaterial` is the whole write side: it puts the material on
 * `<html data-window-material>` and the material block in `App.css` decides
 * which roles go translucent. Nothing else in the frontend asks.
 *
 * A window that was built opaque can never show a backdrop, so the startup
 * decision also travels ahead of this module as an initialization script that
 * `index.html` reads — the pre-React splash has to agree with it, or the canvas
 * it paints covers the material for as long as the bundle takes to load. What
 * this module does is keep the attribute in step with the platform afterwards.
 *
 * The theme goes the other way and is the one thing this module *writes* to
 * the backend: both backdrops tint themselves from the system appearance, which
 * is not necessarily the one the app is pinned to, so `syncWindowMaterialTheme`
 * reports which appearance the shell is actually drawing in.
 */

import { commands, events, type Material } from "@/bindings";

/** Mirrors the backdrop onto `<html data-window-material>` for CSS to read —
 *  the material block in `App.css` is its only consumer. `Material::None` means
 *  "there is nothing behind this window" and is drawn as such: the shell paints
 *  its own canvas, exactly as it did before any of this existed. */
export function applyWindowMaterial(material: Material): void {
  document.documentElement.dataset.windowMaterial = material;
}

/**
 * Tells the platform which appearance this shell is drawing in, so the backdrop
 * matches it instead of the system's.
 *
 * The native command is a no-op while there is no backdrop to tint, so this is
 * safe to call on every theme change and on every window, including one that
 * will never show Mica.
 */
export function syncWindowMaterialTheme(): void {
  const dark = document.documentElement.classList.contains("dark");
  void commands.setWindowMaterialAppearance(dark).catch((error: unknown) => {
    console.warn("Unable to tint the window backdrop", error);
  });
}

/**
 * Follows the OS backdrop and hands each reading to `onChange`.
 *
 * The read side of the seam, and the same subscribe-before-pull order
 * `watchSystemAccent` uses: the watcher thread starts before the webview mounts
 * a listener, so its opening report can already have gone by the time this
 * runs. Listening first closes that gap, and the pull that follows is a
 * snapshot rather than the only source of truth.
 */
export function watchWindowMaterial(onChange: (material: Material) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;

  const start = async () => {
    unlisten = await events.windowMaterialChanged.listen(({ payload }) => {
      if (!disposed) onChange(payload);
    });
    if (disposed) {
      unlisten();
      unlisten = null;
      return;
    }

    onChange(await commands.getWindowMaterial());
  };

  void start().catch((error: unknown) => {
    console.warn("Unable to follow the window material", error);
  });

  return () => {
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}
