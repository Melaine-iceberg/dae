import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ClipboardTextIcon,
  CopyIcon,
  FolderOpenIcon,
  ScissorsIcon,
  StarIcon,
} from "@phosphor-icons/react";
import { useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import type { ComponentProps, ReactNode } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { fileClipboardAtom, openInNewTabAtom } from "@/features/explorer/tabs";

import { addFavoritePathsAtom } from "./sidebar-atoms";

/**
 * The shared right-click menu for a folder path in the sidebar: open in a new
 * tab, favorite, copy the path, and queue copy/cut transfers. `isListed`
 * hides the favorite action for entries that already appear in Favorites.
 */
export function FolderContextMenu({
  children,
  isListed,
  path,
  triggerProps,
}: {
  children: ReactNode;
  isListed: boolean;
  path: string;
  triggerProps?: ComponentProps<typeof ContextMenuTrigger>;
}) {
  const { t } = useTranslation("sidebar");
  const setClipboard = useSetAtom(fileClipboardAtom);
  const addFavoritePaths = useSetAtom(addFavoritePathsAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);

  return (
    <ContextMenu>
      <ContextMenuTrigger {...triggerProps}>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => openInNewTab(path)}>
            <FolderOpenIcon />
            {t("contextMenu.openInNewTab")}
          </ContextMenuItem>
          {!isListed && (
            <ContextMenuItem onClick={() => addFavoritePaths([path])}>
              <StarIcon />
              {t("contextMenu.addFavorite")}
            </ContextMenuItem>
          )}
          <ContextMenuItem onClick={() => void copyEntryPath(path)}>
            <ClipboardTextIcon />
            {t("contextMenu.copyFilePath")}
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => setClipboard({ operation: "copy", sourcePaths: [path] })}>
            <CopyIcon />
            {t("contextMenu.copy")}
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setClipboard({ operation: "cut", sourcePaths: [path] })}>
            <ScissorsIcon />
            {t("contextMenu.cut")}
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export async function copyEntryPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
  }
}
