/**
 * The UI *is* the webview's own document, so a right-click on any pixel that
 * no app menu claims opens the webview's built-in page menu ("Refresh",
 * "Save as", "Print", "Inspect"), which has nothing to do with a file
 * manager. Base UI's `ContextMenuTrigger` suppresses it on the surfaces that
 * own a menu (entries, places, connections), but that leaves every other
 * region: title bar, tab strip, status bar, empty panels, dialogs, panel
 * splitters, the terminal grid.
 *
 * Two contexts keep the OS menu, because the app has no replacement for it:
 * - Form fields: undo / cut / copy / paste / select all live nowhere else.
 * - Anywhere with a live text selection (e.g. the properties dialog's
 *   copyable values), so "Copy" stays one right-click away.
 */
const EDITABLE_SELECTOR = "input, textarea, [contenteditable]:not([contenteditable='false'])";

export function setupNativeContextMenuGuard(): void {
  document.addEventListener(
    "contextmenu",
    (event) => {
      const target = event.target;
      if (target instanceof Element) {
        if (target.closest(EDITABLE_SELECTOR)) return;
        if (hasTextSelection(target)) return;
      }

      event.preventDefault();
    },
    // Capture phase: the guard has to win even for a component that stops
    // propagation from its own `contextmenu` handler. Preventing the default
    // never blocks the app's own menus — they open from the same event.
    true,
  );
}

/** Whether the selection the user would expect "Copy" to act on lives inside `target`. */
function hasTextSelection(target: Element): boolean {
  const selection = target.ownerDocument.getSelection();
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return false;

  const anchor = selection.anchorNode;
  return anchor !== null && target.contains(anchor);
}
