import {
  ClipboardTextIcon,
  CopyIcon,
  FolderOpenIcon,
  PencilIcon,
  ScissorsIcon,
  StarIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import {
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
} from "@/components/ui/context-menu";

import type { DirectoryEntry } from "./types";

export interface EntryActions {
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isSingleSelection: boolean;
  onAddToFavorites: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpen: () => void;
  onRename: () => void;
}

export function EntryContextMenuContent({
  entry,
  isActionDisabled,
  isSingleSelection,
  onAddToFavorites,
  onCopy,
  onCut,
  onDelete,
  onOpen,
  onRename,
}: EntryActions) {
  return (
    <>
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onOpen}>
          <FolderOpenIcon />
          打开
          <ContextMenuShortcut>Enter</ContextMenuShortcut>
        </ContextMenuItem>
        {entry.kind === "directory" && (
          <ContextMenuItem disabled={isActionDisabled} onClick={onAddToFavorites}>
            <StarIcon />
            添加到常用位置
          </ContextMenuItem>
        )}
        <ContextMenuItem disabled={isActionDisabled || !isSingleSelection} onClick={onRename}>
          <PencilIcon />
          重命名
          <ContextMenuShortcut>F2</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onClick={() => void copyEntryPath(entry.path)}>
          <ClipboardTextIcon />
          复制文件地址
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCopy}>
          <CopyIcon />
          复制
          <ContextMenuShortcut>Ctrl+C</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCut}>
          <ScissorsIcon />
          剪切
          <ContextMenuShortcut>Ctrl+X</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onDelete} variant="destructive">
          <TrashIcon />
          删除
          <ContextMenuShortcut>Delete</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuGroup>
    </>
  );
}

async function copyEntryPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
  }
}
