import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  ArrowsOutCardinalIcon,
  ClipboardTextIcon,
  CopyIcon,
  FileZipIcon,
  FilesIcon,
  FolderOpenIcon,
  PencilIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { MOD_KEY } from "@/lib/platform";

import {
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";

import type { DirectoryEntry } from "./types";

export interface EntryActions {
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isSingleSelection: boolean;
  onAddToFavorites: () => void;
  onAddToSpace: (spaceId: string) => void;
  onCompress: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onMoveTo: () => void;
  onOpen: () => void;
  onRename: () => void;
}

export function EntryContextMenuContent({
  entry,
  isActionDisabled,
  isSingleSelection,
  onAddToFavorites,
  onAddToSpace,
  onCompress,
  onCopy,
  onCut,
  onDelete,
  onDuplicate,
  onMoveTo,
  onOpen,
  onRename,
}: EntryActions) {
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

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
            添加到收藏
          </ContextMenuItem>
        )}
        {entry.kind === "directory" && spaces.length > 0 && (
          <ContextMenuSub>
            <ContextMenuSubTrigger disabled={isActionDisabled}>
              <SquaresFourIcon />
              添加到空间
            </ContextMenuSubTrigger>
            <ContextMenuSubContent>
              {spaces.map((space) => (
                <ContextMenuItem key={space.id} onClick={() => onAddToSpace(space.id)}>
                  <SquaresFourIcon />
                  {space.name}
                </ContextMenuItem>
              ))}
            </ContextMenuSubContent>
          </ContextMenuSub>
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
        <ContextMenuItem disabled={isActionDisabled} onClick={onDuplicate}>
          <FilesIcon />
          创建副本
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCompress}>
          <FileZipIcon />
          压缩为ZIP
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled} onClick={onMoveTo}>
          <ArrowsOutCardinalIcon />
          移动到…
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCopy}>
          <CopyIcon />
          复制
          <ContextMenuShortcut>{MOD_KEY}+C</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCut}>
          <ScissorsIcon />
          剪切
          <ContextMenuShortcut>{MOD_KEY}+X</ContextMenuShortcut>
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
