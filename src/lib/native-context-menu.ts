/**
 * The UI *is* the webview's own document, so a right-click on any pixel would
 * otherwise open WebView2's built-in page menu ("Refresh", "Save as", "Print",
 * "Inspect"), which has nothing to do with a file manager.
 *
 * The guard owns one rule: the webview menu never appears. What replaces it
 * depends on the surface:
 * - Surfaces with a richer menu of their own (file entries, places, the
 *   terminal grid, the breadcrumb bar, connections) use Base UI's
 *   `ContextMenuTrigger`, which claims the same event and renders that menu.
 * - Form fields and live text selections are served by the app's own text
 *   menu — `<TextContextMenu/>` registers itself here.
 * - Anywhere else the menu is simply suppressed.
 */
type AppContextMenuOpener = (event: MouseEvent) => boolean;

let openAppContextMenu: AppContextMenuOpener | null = null;

/**
 * Registers the app-rendered replacement for the webview's text menu. Returns
 * a disposer that only clears the registration it installed. Until the React
 * tree mounts, the guard just suppresses the webview menu.
 */
export function registerAppContextMenu(opener: AppContextMenuOpener): () => void {
  openAppContextMenu = opener;
  return () => {
    if (openAppContextMenu === opener) openAppContextMenu = null;
  };
}

export function setupNativeContextMenuGuard(): void {
  document.addEventListener(
    "contextmenu",
    (event) => {
      // The app menu opens only where it has something to offer; the webview
      // menu is suppressed either way.
      openAppContextMenu?.(event);
      event.preventDefault();
    },
    // Capture phase: the guard has to win even for a component that stops
    // propagation from its own `contextmenu` handler. Preventing the default
    // never blocks the app's own menus — they open from the same event.
    true,
  );
}
