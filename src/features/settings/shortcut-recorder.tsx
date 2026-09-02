/**
 * Inline keyboard-shortcut recorder for the settings dialog.
 *
 * A row shows its current binding; clicking it enters capture mode, which
 * pauses every app hotkey (via {@link hotkeysPausedAtom}) so pressing e.g.
 * `Mod+C` records the combo instead of also running Copy. The next complete
 * keystroke is normalized to TanStack's canonical form and committed:
 *
 * - `Escape` cancels capture.
 * - `Backspace` / `Delete` with no modifier unbinds the action (empty string).
 * - A combo that another action already uses triggers a confirm-to-replace
 *   step rather than silently stealing the binding.
 *
 * Capture uses a window-level listener in the capture phase and always calls
 * `preventDefault`/`stopPropagation` so the keystroke never reaches the file
 * list, the command bar or the browser underneath the dialog.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { ArrowCounterClockwiseIcon } from "@phosphor-icons/react";
import { hasNonModifierKey, normalizeHotkey, normalizeHotkeyFromEvent } from "@tanstack/react-hotkeys";

import { appSettingsAtom, hotkeysPausedAtom } from "./settings-atoms";
import {
  DEFAULT_BINDINGS,
  formatBinding,
  resolveBinding,
  type ShortcutId,
} from "./shortcut-registry";
import { cn } from "@/lib/utils";

type Mode = "idle" | "capturing" | "confirm";

export interface ShortcutRecorderProps {
  id: ShortcutId;
  /** The row's current effective binding (already resolved). */
  binding: string;
  onCommit: (next: string) => void;
  onReset: () => void;
}

/** True when the event carries any modifier, so a lone Backspace unbinds but
 *  `Mod+Backspace` is treated as a real combo. */
function hasModifier(event: KeyboardEvent): boolean {
  return event.ctrlKey || event.metaKey || event.altKey || event.shiftKey;
}

export function ShortcutRecorder({ binding, id, onCommit, onReset }: ShortcutRecorderProps) {
  const { t } = useTranslation("settings");
  const setPaused = useSetAtom(hotkeysPausedAtom);
  const shortcuts = useAtomValue(appSettingsAtom)?.shortcuts;

  const [mode, setMode] = useState<Mode>("idle");
  const [pending, setPending] = useState<string | null>(null);
  const [conflictId, setConflictId] = useState<ShortcutId | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  /** First *other* action whose normalized binding equals `candidate`. */
  const findConflict = useCallback(
    (candidate: string): ShortcutId | null => {
      const normalized = normalizeHotkey(candidate);
      for (const other of Object.keys(DEFAULT_BINDINGS) as ShortcutId[]) {
        if (other === id) continue;
        if (normalizeHotkey(resolveBinding(shortcuts, other)) === normalized) return other;
      }
      return null;
    },
    [id, shortcuts],
  );

  const stopCapture = useCallback(() => {
    setMode("idle");
    setPaused(false);
  }, [setPaused]);

  const commit = useCallback(
    (next: string) => {
      onCommit(next);
      stopCapture();
    },
    [onCommit, stopCapture],
  );

  useEffect(() => {
    if (mode !== "capturing") return;
    setPaused(true);

    const onKeyDown = (event: KeyboardEvent) => {
      // Swallow the keystroke unconditionally while recording: nothing beneath
      // the dialog should react to a combo the user is binding.
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        stopCapture();
        return;
      }
      if ((event.key === "Backspace" || event.key === "Delete") && !hasModifier(event)) {
        commit("");
        return;
      }
      // Ignore modifier-only presses; wait for a real key to complete the combo.
      if (!hasNonModifierKey(normalizeHotkeyFromEvent(event))) return;

      const candidate = normalizeHotkeyFromEvent(event);
      const conflict = findConflict(candidate);
      if (conflict) {
        setPending(candidate);
        setConflictId(conflict);
        setMode("confirm");
        return;
      }
      commit(candidate);
    };

    window.addEventListener("keydown", onKeyDown, true);
    buttonRef.current?.focus();
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      setPaused(false);
    };
  }, [commit, findConflict, mode, setPaused, stopCapture]);

  const capturing = mode === "capturing";
  const confirming = mode === "confirm";

  return (
    <div className="flex items-center justify-end gap-1.5">
      {confirming && conflictId ? (
        <div className="flex items-center gap-1.5">
          <span className="max-w-56 truncate text-xs text-muted-foreground">
            {t("recorder.conflict", { action: t(`actions.${conflictId}`) })}
          </span>
          <button
            className="h-6 rounded-sm border border-input px-2 text-xs font-medium transition-colors hover:bg-accent"
            onClick={() => pending != null && commit(pending)}
            type="button"
          >
            {t("recorder.replace")}
          </button>
          <button
            className="h-6 rounded-sm px-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={stopCapture}
            type="button"
          >
            {t("recorder.cancel")}
          </button>
        </div>
      ) : (
        <>
          <button
            aria-label={t("recorder.changeAria", { action: t(`actions.${id}`) })}
            className={cn(
              "flex h-7 min-w-24 items-center justify-center rounded-sm border px-2 font-mono text-xs transition-colors",
              capturing
                ? "border-ring bg-primary-container text-on-primary-container ring-2 ring-ring/40"
                : "border-input text-foreground hover:bg-accent",
            )}
            onClick={() => setMode(capturing ? "idle" : "capturing")}
            ref={buttonRef}
            type="button"
          >
            {capturing ? t("recorder.listening") : formatBinding(binding)}
          </button>
          <button
            aria-label={t("recorder.resetAria", { action: t(`actions.${id}`) })}
            className="flex size-7 items-center justify-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
            disabled={binding === DEFAULT_BINDINGS[id]}
            onClick={onReset}
            title={t("recorder.reset")}
            type="button"
          >
            <ArrowCounterClockwiseIcon className="size-3.5" />
          </button>
        </>
      )}
    </div>
  );
}
