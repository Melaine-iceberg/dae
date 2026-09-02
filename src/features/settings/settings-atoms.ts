/**
 * Frontend state for the settings panel.
 *
 * The Rust backend (`settings.toml`) is the single source of truth — there is
 * deliberately no `atomWithStorage` mirror here. On mount we hydrate once via
 * `commands.loadSettings()`; every mutation is optimistic (the atom updates
 * immediately) followed by a fire-and-forget `saveSettings` that reconciles the
 * atom with the backend-normalized value.
 *
 * `CLIENT_DEFAULT_SETTINGS` mirrors the Rust defaults so the very first paint
 * (before hydration resolves) already shows correct bindings and terminal
 * typography instead of a flash of empty state.
 */

import { atom, useAtom, useAtomValue, useSetAtom } from "jotai";
import { useCallback, useEffect } from "react";

import { commands, type AppSettings } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { DEFAULT_BINDINGS, type ShortcutId } from "./shortcut-registry";

/** Whether the settings dialog is open. */
export const settingsOpenAtom = atom(false);

/**
 * Set while the shortcut recorder is capturing a new binding, so every app
 * hotkey goes inert (via each registration's `enabled`) and pressing e.g.
 * `Mod+C` records the combo instead of also running the copy action.
 */
export const hotkeysPausedAtom = atom(false);

/** Hydrated settings, or `null` before the first successful load. */
export const appSettingsAtom = atom<AppSettings | null>(null);

/** TS mirror of the Rust `AppSettings::default()` for first paint. */
export const CLIENT_DEFAULT_SETTINGS: AppSettings = {
  shortcuts: { ...DEFAULT_BINDINGS },
  terminal: { fontSize: 13, lineHeight: 1.2, fontFamily: null, ansiColors: null },
  defaultFileManager: { isDefault: false },
};

/**
 * Shallow-merges a patch into the base settings. `shortcuts` is treated as a
 * delta (id -> binding) so callers can update one action without resending the
 * whole map; `terminal` and `defaultFileManager` merge field-by-field.
 */
function mergeSettings(base: AppSettings, patch: Partial<AppSettings>): AppSettings {
  return {
    ...base,
    ...(patch.shortcuts ? { shortcuts: { ...base.shortcuts, ...patch.shortcuts } } : null),
    ...(patch.terminal ? { terminal: { ...base.terminal, ...patch.terminal } } : null),
    ...(patch.defaultFileManager
      ? { defaultFileManager: { ...base.defaultFileManager, ...patch.defaultFileManager } }
      : null),
  };
}

/**
 * Reads the live settings (falling back to client defaults pre-hydration) and
 * returns an optimistic `patch` that persists through the backend.
 */
export function useSettings(): readonly [AppSettings, (patch: Partial<AppSettings>) => void] {
  const [stored, setStored] = useAtom(appSettingsAtom);
  const settings = stored ?? CLIENT_DEFAULT_SETTINGS;

  const patch = useCallback(
    (partial: Partial<AppSettings>) => {
      const next = mergeSettings(settings, partial);
      setStored(next);
      void commands
        .saveSettings(next)
        .then((normalized) => setStored(normalized))
        .catch((error) => {
          console.error("Failed to save settings:", getFileOperationErrorMessage(error));
        });
    },
    [settings, setStored],
  );

  return [settings, patch] as const;
}

/** The current binding for an action id, or its default before hydration. */
export function useBinding(id: ShortcutId): string {
  const settings = useAtomValue(appSettingsAtom);
  return settings?.shortcuts?.[id] ?? DEFAULT_BINDINGS[id] ?? "";
}

/** Loads persisted settings once on mount; call from the app root. */
export function useHydrateSettings(): void {
  const setStored = useSetAtom(appSettingsAtom);

  useEffect(() => {
    let cancelled = false;
    void commands
      .loadSettings()
      .then((loaded) => {
        if (!cancelled) setStored(loaded);
      })
      .catch((error) => {
        console.error("Failed to load settings:", getFileOperationErrorMessage(error));
        if (!cancelled) setStored(CLIENT_DEFAULT_SETTINGS);
      });
    return () => {
      cancelled = true;
    };
  }, [setStored]);
}
