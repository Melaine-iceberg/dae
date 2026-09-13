import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ClipboardList, Copy, Pencil, SquareTerminal } from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { commands } from "@/bindings";

import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
import { focusAfterPopupClose } from "@/lib/dom";
import { cn } from "@/lib/utils";
import type { Breadcrumb, DirectoryView } from "./types";

interface ExplorerPathBarProps {
  directory: DirectoryView;
  onNavigate: (breadcrumb: Breadcrumb) => void;
  onNavigatePath: (path: string) => Promise<boolean>;
}

export function ExplorerPathBar({ directory, onNavigate, onNavigatePath }: ExplorerPathBarProps) {
  const { t } = useTranslation("explorer");
  const [isEditing, setIsEditing] = useState(false);
  const [value, setValue] = useState("");
  const [isInvalid, setIsInvalid] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const pathBarRef = useRef<HTMLDivElement>(null);
  const currentName = directory.breadcrumbs.at(-1)?.name ?? directory.path;

  useEffect(() => {
    setIsEditing(false);
    setIsInvalid(false);
  }, [directory.path]);

  useEffect(() => {
    if (!isEditing) return;
    const input = inputRef.current;
    input?.focus();
    input?.select();
  }, [isEditing]);

  /** Keeps the caret on the bar the menu belongs to, instead of handing it
   *  back to whatever field happened to be focused before the menu opened
   *  (the toolbar search box is the usual culprit). */
  const focusPathBar = () => {
    focusAfterPopupClose(pathBarRef.current);
  };

  // Opens the path editor seeded with the current path. Shared by the click
  // handler and the context menu's "edit path" item.
  const beginEditing = () => {
    setValue(directory.path);
    setIsInvalid(false);
    setIsEditing(true);
  };

  const copyPath = () => {
    void copyText(directory.path);
    focusPathBar();
  };

  const copyName = () => {
    void copyText(currentName);
    focusPathBar();
  };

  const openInTerminal = () => {
    void openTerminalAt(directory.path);
    focusPathBar();
  };

  const startEditing = (event: MouseEvent) => {
    if (event.defaultPrevented) return;
    beginEditing();
  };

  const submitPath = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const path = value.trim();
    if (!path || path === directory.path) {
      setIsEditing(false);
      return;
    }

    const succeeded = await onNavigatePath(path);
    if (!succeeded) {
      setIsInvalid(true);
      inputRef.current?.focus();
    }
  };

  if (isEditing) {
    return (
      <form
        className={cn(
          "flex h-8 min-w-0 flex-1 items-center rounded-full border bg-muted/70 pr-3 pl-3.5 transition-[background-color,border-color,box-shadow] focus-within:bg-card focus-within:ring-2 focus-within:ring-ring/30",
          isInvalid ? "border-destructive" : "border-transparent focus-within:border-ring",
        )}
        onSubmit={(event) => void submitPath(event)}
      >
        <input
          ref={inputRef}
          aria-invalid={isInvalid}
          aria-label={t("pathBar.currentPath")}
          className="h-full min-w-0 flex-1 bg-transparent text-[13px] outline-none"
          onChange={(event) => {
            setValue(event.target.value);
            setIsInvalid(false);
          }}
          onBlur={() => {
            setIsEditing(false);
            setIsInvalid(false);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setIsEditing(false);
            }
          }}
          spellCheck={false}
          value={value}
        />
      </form>
    );
  }

  return (
    <ContextMenu>
      <ContextMenuTrigger
        // `select-text` restores text selection inside the bar, which the
        // shared trigger class turns off. `tabIndex` makes the bar itself
        // focusable so a menu action can keep the caret here.
        className="flex h-8 min-w-0 flex-1 select-text items-center rounded-full border border-transparent bg-muted/70 px-3.5 transition-colors hover:bg-muted dark:bg-muted/50 dark:hover:bg-muted/70"
        data-tauri-drag-region="false"
        onClick={startEditing}
        ref={pathBarRef}
        tabIndex={-1}
        title={t("pathBar.clickToEdit")}
      >
        <ExplorerBreadcrumbs breadcrumbs={directory.breadcrumbs} onNavigate={onNavigate} />
      </ContextMenuTrigger>
      <ContextMenuContent
        // This menu never hands focus back on close: "edit path" wants the
        // freshly mounted editor to keep it, and the other items keep it on
        // the bar (see `focusPathBar`) rather than on an unrelated field.
        finalFocus={false}
        className="min-w-44"
      >
        <ContextMenuGroup>
          <ContextMenuItem onClick={copyPath}>
            <ClipboardList />
            {t("contextMenu.copyPath")}
          </ContextMenuItem>
          <ContextMenuItem onClick={copyName}>
            <Copy />
            {t("contextMenu.copyName")}
          </ContextMenuItem>
          <ContextMenuItem onClick={openInTerminal}>
            <SquareTerminal />
            {t("contextMenu.openInTerminal")}
          </ContextMenuItem>
          <ContextMenuItem onClick={beginEditing}>
            <Pencil />
            {t("contextMenu.editPath")}
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** Copies text to the system clipboard; failures are logged, never thrown. */
async function copyText(text: string): Promise<void> {
  try {
    await writeText(text);
  } catch (error) {
    console.warn(`Unable to copy ${text}`, error);
  }
}

/** Opens the system terminal at `path`; failures are logged, never thrown. */
async function openTerminalAt(path: string): Promise<void> {
  try {
    await commands.openTerminal(path);
  } catch (error) {
    console.warn(`Unable to open a terminal at ${path}`, error);
  }
}
