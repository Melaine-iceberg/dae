import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  AppWindowIcon,
  ArchiveTrayIcon,
  ArrowsOutCardinalIcon,
  ClipboardTextIcon,
  CopyIcon,
  FileZipIcon,
  FilesIcon,
  FolderOpenIcon,
  InfoIcon,
  PencilIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
  TerminalIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands, type ArchiveFormat } from "@/bindings";
import { isWindowsPlatform, MOD_KEY } from "@/lib/platform";

import { propertiesTargetAtom } from "./properties-dialog";

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

const COMPRESS_FORMATS: { format: ArchiveFormat; labelKey: string }[] = [
  { format: "zip", labelKey: "zip" },
  { format: "tar", labelKey: "tar" },
  { format: "tar.gz", labelKey: "tarGz" },
  { format: "7z", labelKey: "7z" },
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
  const { t } = useTranslation("explorer");
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const setPropertiesTarget = useSetAtom(propertiesTargetAtom);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  return (
    <>
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onOpen}>
          <FolderOpenIcon />
          {t("explorer:contextMenu.open")}
          <ContextMenuShortcut>Enter</ContextMenuShortcut>
        </ContextMenuItem>
        {entry.kind === "file" && isWindowsPlatform && (
          <ContextMenuItem
            disabled={isActionDisabled}
            onClick={() => void openWithSystemDialog(entry.path)}
          >
            <AppWindowIcon />
            {t("explorer:contextMenu.openWith")}
          </ContextMenuItem>
        )}
        {entry.kind === "directory" && (
          <ContextMenuItem disabled={isActionDisabled} onClick={onAddToFavorites}>
            <StarIcon />
            {t("explorer:contextMenu.addToFavorites")}
          </ContextMenuItem>
        )}
        {entry.kind === "directory" && spaces.length > 0 && (
          <ContextMenuSub>
            <ContextMenuSubTrigger disabled={isActionDisabled}>
              <SquaresFourIcon />
              {t("explorer:contextMenu.addToSpace")}
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
          {t("explorer:contextMenu.rename")}
          <ContextMenuShortcut>F2</ContextMenuShortcut>
        </ContextMenuItem>
        {entry.kind === "directory" && (
          <ContextMenuItem disabled={isActionDisabled} onClick={() => void openTerminalAt(entry.path)}>
            <TerminalIcon />
            {t("explorer:contextMenu.openInTerminal")}
          </ContextMenuItem>
        )}
        <ContextMenuItem onClick={() => void copyEntryPath(entry.path)}>
          <ClipboardTextIcon />
          {t("explorer:contextMenu.copyPath")}
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onDuplicate}>
          <FilesIcon />
          {t("explorer:contextMenu.duplicate")}
        </ContextMenuItem>
        <ContextMenuSub>
          <ContextMenuSubTrigger disabled={isActionDisabled}>
            <FileZipIcon />
            {t("explorer:contextMenu.compressAs")}
          </ContextMenuSubTrigger>
          <ContextMenuSubContent>
            {COMPRESS_FORMATS.map(({ format, labelKey }) => (
              <ContextMenuItem
                key={format}
                disabled={isActionDisabled}
                onClick={() => onCompress(format)}
              >
                <FileZipIcon />
                {t(`explorer:compressFormats.${labelKey}`)}
              </ContextMenuItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
        {isArchiveFile(entry) && (
          <ContextMenuItem disabled={isActionDisabled} onClick={() => onExtract(entry.path)}>
            <ArchiveTrayIcon />
            {t("explorer:contextMenu.extractHere")}
          </ContextMenuItem>
        )}
        <ContextMenuItem disabled={isActionDisabled} onClick={onMoveTo}>
          <ArrowsOutCardinalIcon />
          {t("explorer:contextMenu.moveTo")}
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCopy}>
          <CopyIcon />
          {t("explorer:contextMenu.copy")}
          <ContextMenuShortcut>{MOD_KEY}+C</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled} onClick={onCut}>
          <ScissorsIcon />
          {t("explorer:contextMenu.cut")}
          <ContextMenuShortcut>{MOD_KEY}+X</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onDelete} variant="destructive">
          <TrashIcon />
          {t("explorer:contextMenu.delete")}
          <ContextMenuShortcut>Delete</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem
          disabled={isActionDisabled || !isSingleSelection}
          onClick={() => setPropertiesTarget(entry)}
        >
          <InfoIcon />
          {t("explorer:contextMenu.properties")}
          <ContextMenuShortcut>Alt+Enter</ContextMenuShortcut>
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

async function openWithSystemDialog(path: string): Promise<void> {
  try {
    await commands.openWith(path);
  } catch (error) {
    console.warn(`Unable to open "open with" picker for ${path}`, error);
  }
}
