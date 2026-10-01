import type { ReactNode } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { toast } from "sonner";

import { Kbd } from "@/components/ui/kbd";
import { i18n } from "@/i18n";

/**
 * The app's notification vocabulary — the only module that talks to Sonner.
 *
 * Every transient message dae raises goes through here, so that "how long does
 * this live", "does it carry a close button", "what shape is its action" are
 * answered once instead of at forty call sites. The host that renders these is
 * `@/components/ui/sonner`; the look is in App.css. What is left for this file
 * is the grammar:
 *
 *   success     an outcome with nothing left to do about it ("已复制地址")
 *   error       a failure the user may need to read twice, so it lingers and
 *               keeps its dismiss plate
 *   info        context that is not a result of anything
 *   withAction  an outcome that can still be taken back — the delete/undo pair
 *               being the reason this exists at all
 *
 * Two conventions are load-bearing:
 *
 * - MESSAGES ARRIVE ALREADY TRANSLATED. Call sites are components, they hold a
 *   `t`, and their wording belongs in their own feature's namespace next to the
 *   button that produced it. The two exceptions are the ones with no component
 *   to translate from (see `copyWithNotice`), and they read the i18next
 *   singleton directly.
 * - FAILURES CARRY THE BACKEND'S ALREADY-LOCALIZED DETAIL as the body, under a
 *   short title. That is the same split the banner it replaced used: the title
 *   says which step failed, the body says what the OS said.
 */

/** A settled outcome needs long enough to be read, and no longer. */
const SUCCESS_DURATION_MS = 4000;

/**
 * A failure is the one message worth keeping on screen: the user may have to
 * carry the detail into a search box, and auto-dismissal mid-read is the
 * complaint that produced the old banner's explicit close button.
 */
const ERROR_DURATION_MS = 8000;

/**
 * The undo window, matching the one the explorer advertises. A message whose
 * whole point is its button must outlive the time it takes to move the pointer
 * to it; Sonner pauses the timer while the pointer is over the stack, so this
 * is the floor rather than the deadline.
 */
const ACTION_DURATION_MS = 6000;

/** One step offered on a notification, with its live keycap. */
export type NotifyAction = {
  /** The verb, already localized — "撤销", "重做". */
  label: string;
  /** The binding that does the same thing, already formatted for display. It is
   *  read from the registry at render time, so a rebound key never leaves the
   *  notification teaching a chord that no longer works. */
  hint?: string;
  onClick: () => void;
};

/** Sonner's toast handle, kept opaque. */
export type NotifyHandle = string | number;

function actionOf(action: NotifyAction): { label: ReactNode; onClick: () => void } {
  return {
    label: (
      <span className="flex items-center gap-1">
        {action.label}
        {action.hint && <Kbd className="h-4 px-1 text-nano">{action.hint}</Kbd>}
      </span>
    ),
    onClick: action.onClick,
  };
}

export const notify = {
  /** An outcome with nothing left to do about it. */
  success(message: string): NotifyHandle {
    return toast.success(message, { closeButton: false, duration: SUCCESS_DURATION_MS });
  },

  /**
   * A failure. `title` names the step that failed and `message` becomes the
   * detail under it; without a title the message stands alone, which is what a
   * message that already reads as a sentence ("撤销失败：…") wants.
   *
   * `id` makes a burst of the same failure one notification instead of five —
   * pass a stable string for anything that can fire per item.
   */
  error(
    message: string,
    options: { action?: NotifyAction; id?: string; title?: string } = {},
  ): NotifyHandle {
    const { action, id, title } = options;
    return toast.error(title ?? message, {
      closeButton: true,
      description: title ? message : undefined,
      duration: ERROR_DURATION_MS,
      id,
      ...(action ? { action: actionOf(action) } : {}),
    });
  },

  /** Context that is not the result of anything the user just did. */
  info(message: string): NotifyHandle {
    return toast.info(message, { closeButton: false, duration: SUCCESS_DURATION_MS });
  },

  /**
   * An outcome that can still be taken back: moved to the trash, undone,
   * redone. The action is the message, so it carries no dismiss plate — and
   * `id` lets a second keystroke replace the first notification rather than
   * stack a contradicting pair.
   */
  withAction(message: string, action: NotifyAction, id?: string): NotifyHandle {
    return toast.success(message, {
      action: actionOf(action),
      closeButton: false,
      duration: ACTION_DURATION_MS,
      id,
    });
  },

  /** Takes one notification down early — used to clear a failure the user has
   *  already moved past (a new directory, the next attempt). */
  dismiss(handle: NotifyHandle | null | undefined): void {
    if (handle === null || handle === undefined) return;
    toast.dismiss(handle);
  },
};

/**
 * Copies text to the clipboard and reports the outcome — the one action whose
 * notification has no component to translate from, since it is raised from
 * several flat `writeText` helpers rather than from a surface with a `t`.
 *
 * It exists because those helpers had each grown the same three lines: the
 * copy, a `console.warn`, and nothing the user could see. Text that silently
 * failed to copy is indistinguishable from text that copied — and the next
 * thing the user does is paste it somewhere.
 *
 * The wording is deliberately about the clipboard rather than about what went
 * on it: the same helper serves a path, a file name and a multi-selection, and
 * the menu item that was just clicked already named the thing.
 */
export async function copyWithNotice(text: string, count = 1): Promise<void> {
  try {
    await writeText(text);
    notify.success(
      count > 1
        ? i18n.t("common:notifications.copied.successMany", { count })
        : i18n.t("common:notifications.copied.success"),
    );
  } catch (error) {
    console.warn("Unable to copy to clipboard", error);
    notify.error(i18n.t("common:notifications.copied.failure"));
  }
}
