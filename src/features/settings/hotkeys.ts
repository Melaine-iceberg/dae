import type { HotkeyCallback, RegisterableHotkey, UseHotkeyOptions } from "@tanstack/react-hotkeys";

import { isEditableElement } from "@/lib/dom";

/**
 * Base options shared by every app hotkey registration (App shell, explorer
 * file list, search, split view).
 *
 * These three flags are the crux of preserving the pre-migration behavior:
 *
 * - `preventDefault` / `stopPropagation` are OFF at the library level. The
 *   manager calls both *before* invoking the callback and gates them only on
 *   these flags — never on anything the callback decides. The old hand-written
 *   handlers swallowed a key only when the action actually ran (e.g. `Mod+C`
 *   with nothing selected fell through untouched). To keep that, handlers call
 *   `event.preventDefault()` themselves, after their own guards pass.
 *
 * - `ignoreInputs` is OFF because the app's own {@link isEditableElement}
 *   predicate is the source of truth — it also covers `contenteditable` rename
 *   cells and the terminal's helper textarea, and the old App-shell handler
 *   deliberately did *not* ignore inputs at all. Handlers apply the predicate
 *   explicitly, matching each original site.
 *
 * - `conflictBehavior: "allow"` lets one combo be registered by both split
 *   panes, and tolerates the pre-existing `` Ctrl+` `` overlap between the
 *   integrated-terminal toggle (App) and open-system-terminal (file list). All
 *   matching registrations fire and each callback self-gates, exactly as the
 *   independent `window` listeners did before, without console warnings.
 *
 * Per-registration `enabled` is layered on top of this for the recorder's
 * global pause. Note that per-definition options *override* `commonOptions`
 * field-by-field rather than merging, so a registration that sets its own
 * `enabled` (e.g. the file list, which gates each action on pane focus and
 * selection) must fold `!hotkeysPaused` into that expression itself.
 */
export const HOTKEY_COMMON_OPTIONS: UseHotkeyOptions = {
  preventDefault: false,
  stopPropagation: false,
  ignoreInputs: false,
  conflictBehavior: "allow",
};

/**
 * Wraps a shortcut action with the guards every explorer/file-list handler
 * shared before the migration: stand down while another handler already
 * prevented default, during IME composition, or when focus is in an editable
 * surface (rename cell, path bar, search field, terminal). The key is swallowed
 * (`preventDefault`) only when the action actually runs, so a combo pressed
 * with nothing selected still falls through to the browser/shell — pass
 * `preventDefault: false` for the rare action (clearing the selection with
 * Escape) that historically never swallowed the key.
 */
export function guardedAction(
  action: () => void,
  { preventDefault = true }: { preventDefault?: boolean } = {},
): HotkeyCallback {
  return (event) => {
    if (event.defaultPrevented || event.isComposing || isEditableElement(event.target)) return;
    if (preventDefault) event.preventDefault();
    action();
  };
}

/**
 * Widens a stored binding string to the library's {@link RegisterableHotkey}.
 *
 * TanStack types a registerable combo as a narrow template-literal union
 * (`Mod+S`, `F2`, …) so literal call sites get autocomplete, but our bindings
 * are arbitrary user-configured strings resolved at runtime. The parser accepts
 * any string, and an empty ("unbound") value simply never matches a keystroke,
 * so the cast is safe and keeps `resolveBinding`/`useBinding` returning plain
 * strings for display via `formatBinding`.
 */
export function asHotkey(binding: string): RegisterableHotkey {
  return binding as RegisterableHotkey;
}
