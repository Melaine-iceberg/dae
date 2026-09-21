import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ClipboardPaste, Copy, Scissors, TextSelect, Trash2 } from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
} from "@/components/ui/context-menu";
import { registerAppContextMenu } from "@/lib/native-context-menu";
import { focusAfterPopupClose } from "@/lib/dom";
import { MOD_KEY } from "@/lib/platform";
import {
  deleteSelection,
  findEditable,
  getSelectedText,
  isReadOnly,
  replaceSelection,
  selectAllText,
} from "@/lib/text-editing";

/**
 * Hover feedback for a menu whose items never take focus (the edited field
 * keeps it): the shared items only style `:focus`, which they would never see.
 */
const ITEM_HOVER = "hover:bg-accent hover:text-accent-foreground";

/**
 * The menu surfaces that must never keep focus: the popup itself (plus every
 * item in it) and the focus guards Base UI wraps around it for Tab trapping.
 */
const MENU_POPUP_SELECTOR = '[data-slot="context-menu-content"], [data-base-ui-focus-guard]';

/** Pointer anchor for a menu that is not attached to a trigger element. */
interface PointerAnchor {
  getBoundingClientRect(): DOMRect;
}

interface TextMenuRequest {
  anchor: PointerAnchor;
  /** The form field the event landed in, or null for a plain text selection. */
  editable: HTMLElement | null;
  hasSelection: boolean;
}

/**
 * App-rendered replacement for the webview's text menu.
 *
 * Sits behind {@link registerAppContextMenu} and opens exactly where the
 * webview menu used to: over form fields and over live text selections.
 * Surfaces with a richer menu (the terminal grid, the breadcrumb bar, file
 * entries) mark their trigger with `data-app-context-menu` and keep their own.
 */
export function TextContextMenu() {
  const { t } = useTranslation("common");
  const [request, setRequest] = useState<TextMenuRequest | null>(null);
  const [open, setOpen] = useState(false);
  // Survives the request reset so focus can return after the exit animation.
  const editableRef = useRef<HTMLElement | null>(null);
  // Why the menu closed: a click elsewhere must keep its own focus target.
  const closeReasonRef = useRef<string | null>(null);

  useEffect(
    () =>
      registerAppContextMenu((event) => {
        const target = event.target;
        if (!(target instanceof Element)) return false;
        // A feature menu owns this event; it renders its own items.
        if (target.closest("[data-app-context-menu]")) return false;

        const editable = findEditable(target);
        const hasSelection = editable
          ? getSelectedText(editable).length > 0
          : hasSelectionInside(target);
        if (!editable && !hasSelection) return false;

        editableRef.current = editable;
        // A keyboard-invoked menu (Menu key / Shift+F10) reports no pointer
        // position; anchor it to the element the user is acting on.
        const bounds = target.getBoundingClientRect();
        const x = event.clientX || bounds.left + bounds.width / 2;
        const y = event.clientY || bounds.top + bounds.height / 2;
        setRequest({
          anchor: { getBoundingClientRect: () => DOMRect.fromRect({ x, y }) },
          editable,
          hasSelection,
        });
        setOpen(true);
        return true;
      }),
    [],
  );

  // Escape closes even after an item has handed focus back to the field it
  // edits ("select all" focuses it), where the popup never sees the keydown.
  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      closeReasonRef.current = "escape-key";
      setOpen(false);
    };
    document.addEventListener("keydown", closeOnEscape, true);
    return () => document.removeEventListener("keydown", closeOnEscape, true);
  }, [open]);

  const editable = request?.editable ?? null;
  // A field only paints its selection while the field itself holds focus, and
  // the popup helps itself to focus when it opens (and when an item is
  // pressed) — which would drop the very selection the menu is acting on. Hold
  // the caret in the field for as long as the menu is up, and only in the field
  // that owns the menu, so clicking into some other one still moves normally.
  // Hover never takes it either: `highlightItemOnHover` is off, so items
  // highlight through `:hover`.
  useEffect(() => {
    if (!open) return;
    const target = editableRef.current;
    if (!target) return;
    const keepFocus = () => {
      if (target.isConnected) target.focus({ preventScroll: true });
    };
    // The popup's own focus is queued on a frame, so undo it after that frame's
    // callbacks have run — and again after a task, for a throttled window where
    // frames stop firing altogether.
    const head = setTimeout(keepFocus, 0);
    let tail: ReturnType<typeof setTimeout> | undefined;
    const frame = requestAnimationFrame(() => {
      keepFocus();
      tail = setTimeout(keepFocus, 0);
    });
    // Later grabs (pressing an item focuses it) get undone from outside the
    // focus event, since a focus() issued inside one is dropped.
    const onFocusIn = (event: FocusEvent) => {
      const next = event.target;
      if (!(next instanceof Element)) return;
      if (!next.closest(MENU_POPUP_SELECTOR)) return;
      setTimeout(keepFocus, 0);
    };
    document.addEventListener("focusin", onFocusIn, true);
    return () => {
      clearTimeout(head);
      if (tail !== undefined) clearTimeout(tail);
      cancelAnimationFrame(frame);
      document.removeEventListener("focusin", onFocusIn, true);
    };
  }, [open]);

  // The field can vanish while the menu is up — submitting the path editor
  // navigates and unmounts it — so close rather than leave a menu anchored to
  // an element that is no longer there.
  useEffect(() => {
    if (!open) return;
    const target = editableRef.current;
    if (!target) return;
    const observer = new MutationObserver(() => {
      if (!target.isConnected) setOpen(false);
    });
    observer.observe(document.body, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [open]);
  /** Ends the menu interaction: every item closes the menu on the click, and
   *  the caret goes back to the field so the user can type or press Enter
   *  next. The menu focuses the pressed item as part of its own click handling,
   *  so the field has to be re-asserted after that, and once more after a task
   *  for a throttled window where neither the frame callback nor the popup's
   *  exit ever run. */
  const returnFocusToField = (target: HTMLElement | null | undefined) => {
    if (!target) return;
    focusAfterPopupClose(target);
    setTimeout(() => {
      if (target.isConnected) target.focus({ preventScroll: true });
    }, 0);
  };

  const copy = () => {
    const target = request?.editable;
    const text = target ? getSelectedText(target) : (window.getSelection()?.toString() ?? "");
    if (text) void writeText(text);
    returnFocusToField(target);
  };

  const cut = () => {
    const target = request?.editable;
    if (!target) return;
    const text = getSelectedText(target);
    if (!text) return;
    void writeText(text);
    deleteSelection(target);
    returnFocusToField(target);
  };

  const paste = () => {
    const target = request?.editable;
    if (!target) return;
    void readText()
      .then((text) => {
        if (text) replaceSelection(target, text);
      })
      .catch((error) => console.warn("Unable to read the clipboard", error));
    // Paste commits the field (it is the item the user reaches for right
    // before typing or pressing Enter), so it closes the menu and puts the
    // caret back.
    returnFocusToField(target);
  };

  const remove = () => {
    const target = request?.editable;
    if (!target) return;
    deleteSelection(target);
    returnFocusToField(target);
  };

  const selectAll = () => {
    const target = request?.editable;
    if (!target) return;
    selectAllText(target);
    // The selected range survives the menu's focus restore, so re-asserting
    // focus is enough to keep the whole value highlighted.
    returnFocusToField(target);
  };

  const writable = editable !== null && !isReadOnly(editable);
  const hasSelection = request?.hasSelection ?? false;
  const shortcut = (key: string) => `${MOD_KEY}+${key}`;

  return (
    <ContextMenu
      open={open}
      // The edited field holds focus while the menu is up (see the focus effect
      // above), so items highlight through `:hover` instead of the `:focus`
      // styling every other menu relies on.
      highlightItemOnHover={false}
      onOpenChange={(next, details) => {
        closeReasonRef.current = details.reason;
        setOpen(next);
      }}
      onOpenChangeComplete={(next) => {
        if (next) return;
        setRequest(null);
        const target = editableRef.current;
        editableRef.current = null;
        if (closeReasonRef.current === "outside-press") return;
        if (target?.isConnected) target.focus({ preventScroll: true });
      }}
    >
      <ContextMenuContent anchor={request?.anchor} className="min-w-menu">
        <ContextMenuGroup>
          <ContextMenuItem className={ITEM_HOVER} disabled={!hasSelection} onClick={copy}>
            <Copy />
            {t("textMenu.copy")}
            <ContextMenuShortcut>{shortcut("C")}</ContextMenuShortcut>
          </ContextMenuItem>
          {editable && (
            <>
              <ContextMenuItem
                className={ITEM_HOVER}
                disabled={!hasSelection || !writable}
                onClick={cut}
              >
                <Scissors />
                {t("textMenu.cut")}
                <ContextMenuShortcut>{shortcut("X")}</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem className={ITEM_HOVER} disabled={!writable} onClick={paste}>
                <ClipboardPaste />
                {t("textMenu.paste")}
                <ContextMenuShortcut>{shortcut("V")}</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem
                className={ITEM_HOVER}
                disabled={!hasSelection || !writable}
                onClick={remove}
              >
                <Trash2 />
                {t("textMenu.delete")}
                <ContextMenuShortcut>Del</ContextMenuShortcut>
              </ContextMenuItem>
            </>
          )}
        </ContextMenuGroup>
        {editable && (
          <>
            <ContextMenuSeparator />
            <ContextMenuGroup>
              <ContextMenuItem className={ITEM_HOVER} onClick={selectAll}>
                <TextSelect />
                {t("textMenu.selectAll")}
                <ContextMenuShortcut>{shortcut("A")}</ContextMenuShortcut>
              </ContextMenuItem>
            </ContextMenuGroup>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** Whether the selection the user expects "copy" to act on lives in `target`. */
function hasSelectionInside(target: Element): boolean {
  const selection = target.ownerDocument.getSelection();
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return false;

  const anchor = selection.anchorNode;
  return anchor !== null && target.contains(anchor);
}
