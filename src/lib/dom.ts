/**
 * Small DOM predicates shared across features.
 */

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
