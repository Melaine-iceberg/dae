/**
 * DOM text-editing helpers shared by the clipboard bridge and the app's own
 * context menus.
 *
 * Writes go through the native value setter and dispatch an `input` event, so
 * a React-controlled field sees them as an ordinary edit (its value tracker
 * notices the change) instead of a state overwrite it would revert.
 */

/** Elements the webview would hand its own text-editing menu to. */
export const EDITABLE_SELECTOR =
  "input, textarea, [contenteditable]:not([contenteditable='false'])";

/** The editable ancestor of a context-menu target, if the target is one. */
export function findEditable(target: EventTarget | null): HTMLElement | null {
  return target instanceof Element ? target.closest<HTMLElement>(EDITABLE_SELECTOR) : null;
}

/** Whether the editable refuses text input (readonly field, disabled control). */
export function isReadOnly(element: HTMLElement): boolean {
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    return element.disabled || element.readOnly;
  }
  return !element.isContentEditable;
}

/** The text "copy"/"cut" should act on for the given event target. */
export function getSelectedText(target: EventTarget | null): string {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    return target.value.slice(target.selectionStart ?? 0, target.selectionEnd ?? 0);
  }

  return window.getSelection()?.toString() ?? "";
}

/** Replaces the current selection with `text`, leaving the caret after it. */
export function replaceSelection(target: HTMLElement, text: string): void {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? start;
    writeControlValue(target, target.value.slice(0, start) + text + target.value.slice(end));
    target.setSelectionRange(start + text.length, start + text.length);
    return;
  }

  if (target.isContentEditable) {
    target.focus();
    document.execCommand("insertText", false, text);
  }
}

/** Removes the current selection (no-op when it is collapsed). */
export function deleteSelection(target: EventTarget | null): void {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? 0;
    const end = target.selectionEnd ?? 0;
    if (start === end) return;

    writeControlValue(target, target.value.slice(0, start) + target.value.slice(end));
    target.setSelectionRange(start, start);
    return;
  }

  if (target instanceof HTMLElement && target.isContentEditable) {
    document.execCommand("delete");
  }
}

/** Selects everything inside `target`. */
export function selectAllText(target: HTMLElement): void {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    target.focus();
    target.select();
    return;
  }

  if (!target.isContentEditable) return;

  const range = document.createRange();
  range.selectNodeContents(target);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

/** Sets a control's value through its native setter so React's value tracker
 *  sees the change, then dispatches the `input` event it listens for. */
function writeControlValue(target: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  const prototype =
    target instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(target, value);
  target.dispatchEvent(new Event("input", { bubbles: true }));
}
