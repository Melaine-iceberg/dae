/**
 * Small DOM helpers shared across features.
 */

/**
 * Focuses `element` now, and again on the next animation frame.
 *
 * Closing a Base UI popup hands focus back to whatever was focused before it
 * opened; that restore is queued as a microtask while the close is committed,
 * so a plain `focus()` from the click handler that triggered the close gets
 * overridden a moment later. Microtasks always run before the next frame, so
 * re-asserting there wins deterministically without racing the popup.
 */
export function focusAfterPopupClose(element: HTMLElement | null | undefined): void {
  if (!element) return;

  element.focus({ preventScroll: true });
  requestAnimationFrame(() => {
    if (element.isConnected) element.focus({ preventScroll: true });
  });
}

/**
 * True when the event originated from a text-entry surface, where global
 * shortcuts must stand down so the user can type freely. Covers plain inputs,
 * textareas, selects, and any `contenteditable` host (inline rename cells,
 * the path bar, the terminal's helper textarea, rich editors).
 *
 * Button-like elements are intentionally *not* editable: a focused button
 * should still let shortcuts such as `Mod+K` or `F2` fire.
 */
export function isEditableElement(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;

  const tagName = target.tagName;
  return (
    target.isContentEditable ||
    tagName === "INPUT" ||
    tagName === "TEXTAREA" ||
    tagName === "SELECT"
  );
}
