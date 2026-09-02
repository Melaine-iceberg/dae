/**
 * Canonical keyboard-shortcut catalogue.
 *
 * The ids and default bindings here MUST mirror `default_shortcuts()` in
 * `src-tauri/src/settings.rs`; a Rust unit test (`default_shortcut_ids_are_stable`)
 * asserts the id set so the two cannot silently drift. Binding strings use
 * TanStack Hotkeys' canonical form: `Mod` is Cmd on macOS and Ctrl elsewhere,
 * `Control` is always Ctrl, and named keys (`Space`, `Delete`, `Escape`, `F2`)
 * or literal punctuation (`` ` ``, `,`) are accepted as-is.
 *
 * This module is pure data + formatting (no jotai) so `settings-atoms.ts` can
 * depend on it without creating an import cycle; the reactive `useBinding`
 * helper lives there.
 */

import { MOD_KEY, isMacPlatform } from "@/lib/platform";

/** Every rebindable action id, grouped by the surface that owns it. */
export type ShortcutId =
  | "app.commandBar"
  | "app.pathJump"
  | "app.toggleTerminal"
  | "app.openSettings"
  | "explorer.clearSelection"
  | "explorer.copy"
  | "explorer.cut"
  | "explorer.paste"
  | "explorer.selectAll"
  | "explorer.toggleHidden"
  | "explorer.rename"
  | "explorer.preview"
  | "explorer.undo"
  | "explorer.redo"
  | "explorer.redoAlt"
  | "explorer.trash"
  | "explorer.deletePermanent"
  | "explorer.openSystemTerminal"
  | "explorer.focusSearch"
  | "explorer.switchPane";

/** Sections shown in the settings dialog's Shortcuts pane. */
export type ShortcutGroup = "app" | "explorer" | "view";

export interface ShortcutAction {
  id: ShortcutId;
  group: ShortcutGroup;
  /** Canonical default binding; also the value a per-row reset restores. */
  defaultBinding: string;
}

/**
 * The full action list, in display order. Order and membership match
 * `default_shortcuts()` in the Rust settings module one-for-one.
 */
export const SHORTCUT_ACTIONS: readonly ShortcutAction[] = [
  // Global (App shell)
  { id: "app.commandBar", group: "app", defaultBinding: "Mod+K" },
  { id: "app.pathJump", group: "app", defaultBinding: "Mod+P" },
  { id: "app.toggleTerminal", group: "app", defaultBinding: "Control+`" },
  { id: "app.openSettings", group: "app", defaultBinding: "Mod+," },
  // Files (explorer operations)
  { id: "explorer.clearSelection", group: "explorer", defaultBinding: "Escape" },
  { id: "explorer.copy", group: "explorer", defaultBinding: "Mod+C" },
  { id: "explorer.cut", group: "explorer", defaultBinding: "Mod+X" },
  { id: "explorer.paste", group: "explorer", defaultBinding: "Mod+V" },
  { id: "explorer.selectAll", group: "explorer", defaultBinding: "Mod+A" },
  { id: "explorer.toggleHidden", group: "explorer", defaultBinding: "Mod+H" },
  { id: "explorer.rename", group: "explorer", defaultBinding: "F2" },
  { id: "explorer.preview", group: "explorer", defaultBinding: "Space" },
  { id: "explorer.undo", group: "explorer", defaultBinding: "Mod+Z" },
  { id: "explorer.redo", group: "explorer", defaultBinding: "Mod+Shift+Z" },
  { id: "explorer.redoAlt", group: "explorer", defaultBinding: "Mod+Y" },
  { id: "explorer.trash", group: "explorer", defaultBinding: "Delete" },
  { id: "explorer.deletePermanent", group: "explorer", defaultBinding: "Shift+Delete" },
  { id: "explorer.openSystemTerminal", group: "explorer", defaultBinding: "Mod+`" },
  // View / navigation
  { id: "explorer.focusSearch", group: "view", defaultBinding: "Mod+F" },
  { id: "explorer.switchPane", group: "view", defaultBinding: "F6" },
] as const;

/** id -> default binding, for first paint and per-row resets. */
export const DEFAULT_BINDINGS: Readonly<Record<ShortcutId, string>> = Object.freeze(
  Object.fromEntries(SHORTCUT_ACTIONS.map((action) => [action.id, action.defaultBinding])) as Record<
    ShortcutId,
    string
  >,
);

/**
 * Resolves the effective binding for an action from a (possibly absent)
 * shortcuts map, falling back to the compiled-in default. Pure, so components
 * that register many hotkeys can read the settings atom once and resolve each
 * id without a hook call per row.
 */
export function resolveBinding(
  shortcuts: Readonly<Record<string, string>> | undefined,
  id: ShortcutId,
): string {
  return shortcuts?.[id] ?? DEFAULT_BINDINGS[id] ?? "";
}

const MODIFIER_LABELS: Record<string, string> = {
  Mod: MOD_KEY,
  Control: isMacPlatform ? "⌃" : "Ctrl",
  Ctrl: isMacPlatform ? "⌃" : "Ctrl",
  Meta: isMacPlatform ? "⌘" : "Win",
  Cmd: isMacPlatform ? "⌘" : "Win",
  Command: isMacPlatform ? "⌘" : "Win",
  Alt: isMacPlatform ? "⌥" : "Alt",
  Option: isMacPlatform ? "⌥" : "Alt",
  Shift: isMacPlatform ? "⇧" : "Shift",
};

const KEY_LABELS: Record<string, string> = {
  Escape: "Esc",
  Space: "Space",
  Delete: "Delete",
  Backspace: "Backspace",
  Enter: "Enter",
  Tab: "Tab",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/**
 * Renders a canonical binding as user-facing hint text, matching the app's
 * existing `Ctrl+C` / `⌘+C` style. An empty or missing binding (unbound)
 * renders as an em dash so the recorder has a stable placeholder.
 */
export function formatBinding(binding: string | null | undefined): string {
  if (!binding) return "—";

  return binding
    .split("+")
    .map((token) => MODIFIER_LABELS[token] ?? KEY_LABELS[token] ?? token)
    .join("+");
}
