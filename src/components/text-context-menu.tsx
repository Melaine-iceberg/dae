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

  const copy = () => {
    const target = request?.editable;
    const text = target ? getSelectedText(target) : (window.getSelection()?.toString() ?? "");
    if (text) void writeText(text);
    focusAfterPopupClose(target);
  };

  const cut = () => {
    const target = request?.editable;
    if (!target) return;
    const text = getSelectedText(target);
    if (!text) return;
    void writeText(text);
    deleteSelection(target);
    focusAfterPopupClose(target);
  };

  const paste = () => {
    const target = request?.editable;
    if (!target) return;
    void readText()
      .then((text) => {
        if (text) replaceSelection(target, text);
      })
      .catch((error) => console.warn("Unable to read the clipboard", error));
    focusAfterPopupClose(target);
  };

  const remove = () => {
    const target = request?.editable;
    if (!target) return;
    deleteSelection(target);
    focusAfterPopupClose(target);
  };

  const selectAll = () => {
    const target = request?.editable;
    if (!target) return;
    selectAllText(target);
    // The selected range survives the menu's focus restore, so re-asserting
    // focus is enough to keep the whole value highlighted.
    focusAfterPopupClose(target);
  };

  const editable = request?.editable ?? null;
  const writable = editable !== null && !isReadOnly(editable);
  const hasSelection = request?.hasSelection ?? false;
  const shortcut = (key: string) => `${MOD_KEY}+${key}`;

  return (
    <ContextMenu
      open={open}
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
      <ContextMenuContent anchor={request?.anchor} className="min-w-44">
        <ContextMenuGroup>
          <ContextMenuItem disabled={!hasSelection} onClick={copy}>
            <Copy />
            {t("textMenu.copy")}
            <ContextMenuShortcut>{shortcut("C")}</ContextMenuShortcut>
          </ContextMenuItem>
          {editable && (
            <>
              <ContextMenuItem disabled={!hasSelection || !writable} onClick={cut}>
                <Scissors />
                {t("textMenu.cut")}
                <ContextMenuShortcut>{shortcut("X")}</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem disabled={!writable} onClick={paste}>
                <ClipboardPaste />
                {t("textMenu.paste")}
                <ContextMenuShortcut>{shortcut("V")}</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem disabled={!hasSelection || !writable} onClick={remove}>
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
              <ContextMenuItem onClick={selectAll}>
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
