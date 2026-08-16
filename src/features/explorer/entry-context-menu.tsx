import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  ArchiveTrayIcon,
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
  TerminalIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands, type ArchiveFormat } from "@/bindings";
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

const ARCHIVE_FILE_PATTERN = /\.(zip|tar|tar\.gz|tgz|7z)$/i;

const COMPRESS_FORMATS: { format: ArchiveFormat; label: string }[] = [
  { format: "zip", label: "ZIP 压缩包" },
  { format: "tar", label: "TAR 归档" },
  { format: "tar.gz", label: "TAR.GZ 压缩包" },
  { format: "7z", label: "7Z 压缩包" },
];

export function isArchiveFile(entry: DirectoryEntry): boolean {
  return entry.kind === "file" && ARCHIVE_FILE_PATTERN.test(entry.name);
}

export interface EntryActions {
  entry: DirectoryEntry;
  isActionDisabled: boolean;
  isSingleSelection: boolean;
  onAddToFavorites: () => void;
  onAddToSpace: (spaceId: string) => void;
  onCompress: (format: ArchiveFormat) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
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
  onExtract,
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
        {entry.kind === "directory" && (
          <ContextMenuItem disabled={isActionDisabled} onClick={() => void openTerminalAt(entry.path)}>
            <TerminalIcon />
            在终端中打开
          </ContextMenuItem>
        )}
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
        <ContextMenuSub>
          <ContextMenuSubTrigger disabled={isActionDisabled}>
            <FileZipIcon />
            压缩为…
          </ContextMenuSubTrigger>
          <ContextMenuSubContent>
            {COMPRESS_FORMATS.map(({ format, label }) => (
              <ContextMenuItem
                key={format}
                disabled={isActionDisabled}
                onClick={() => onCompress(format)}
              >
                <FileZipIcon />
                {label}
              </ContextMenuItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
        {isArchiveFile(entry) && (
          <ContextMenuItem disabled={isActionDisabled} onClick={() => onExtract(entry.path)}>
            <ArchiveTrayIcon />
            解压到此处
          </ContextMenuItem>
        )}
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

async function openTerminalAt(path: string): Promise<void> {
  try {
    await commands.openTerminal(path);
  } catch (error) {
    console.warn(`Unable to open terminal at ${path}`, error);
  }
}
