import { useEffect, useMemo } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  AppWindow,
  PackageOpen,
  Move,
  ClipboardList,
  Copy,
  Eye,
  FileArchive,
  Files,
  FolderOpen,
  Info,
  LockKeyhole,
  PanelsTopLeft,
  Pencil,
  PictureInPicture2,
  Scissors,
  LayoutGrid,
  Star,
  SquareTerminal,
  Trash2,
} from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands, type ArchiveFormat } from "@/bindings";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { ShellCommandsMenu } from "@/features/shell-commands/shell-commands-menu";
import { shellCommandErrorAtom } from "@/features/shell-commands/shell-commands-atoms";

import { propertiesTargetAtom } from "./properties-atoms";
import { openInNewTabAtom, openPathInNewWindowAtom } from "./tabs";

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
import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  removeFavoriteAtom,
} from "@/features/sidebar/sidebar-atoms";

import type { DirectoryEntry } from "./types";

const ARCHIVE_FILE_PATTERN = /\.(zip|tar|tar\.gz|tgz|7z)$/i;

const COMPRESS_FORMATS: { encrypted?: boolean; format: ArchiveFormat; labelKey: string }[] = [
  { format: "zip", labelKey: "zip" },
  { format: "tar", labelKey: "tar" },
  { format: "tar.gz", labelKey: "tarGz" },
  { format: "7z", labelKey: "7z" },
  { encrypted: true, format: "7z", labelKey: "7zEncrypted" },
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
  onCompress: (format: ArchiveFormat, encrypted: boolean) => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpen: () => void;
  onOpenWith: () => void;
  onRename: () => void;
  /**
   * Toggles the preview panel for the right-clicked entry. It is the same
   * action as the toolbar's eye button and the `Space` shortcut, which until
   * now had no menu presence at all — the shortcut existed, the action did
   * not, so nothing in the window taught either.
   */
  onTogglePreview: () => void;
  /**
   * Paths the action would run on — the selection when the right-clicked entry
   * is part of it, otherwise just that entry (the same rule the file operations
   * follow). The shell-command section runs an app's command on exactly these.
   */
  selectedPaths?: readonly string[];
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
  onOpenWith,
  onRename,
  onTogglePreview,
  selectedPaths,
}: EntryActions) {
  const { t } = useTranslation("explorer");
  const previewBinding = formatBinding(useBinding("explorer.preview"));
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const setPropertiesTarget = useSetAtom(propertiesTargetAtom);
  const favorites = useAtomValue(favoritesAtom) ?? [];
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const removeFavorite = useSetAtom(removeFavoriteAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);
  const openInNewWindow = useSetAtom(openPathInNewWindowAtom);
  const setShellCommandError = useSetAtom(shellCommandErrorAtom);
  const isFavorited = favorites.some((favorite) => favorite.path === entry.path);

  useEffect(() => {
    void ensureSpacesLoaded();
    void ensureFavoritesLoaded();
  }, [ensureFavoritesLoaded, ensureSpacesLoaded]);

  // The selection the entry actions run on: the whole selection when the
  // right-clicked entry is part of it, otherwise just that entry.
  const actionPaths = useMemo(
    () =>
      selectedPaths && selectedPaths.length > 0 && selectedPaths.includes(entry.path)
        ? [...selectedPaths]
        : [entry.path],
    [entry.path, selectedPaths],
  );

  return (
    <>
      {/* The entry's own four actions, above everything the menu has to offer.
          They used to sit further down as rows of their own; those rows are
          gone, so no action is reachable two ways from this menu. */}
      <EntryActionRow
        disabled={isActionDisabled}
        onCopy={onCopy}
        onCut={onCut}
        onDelete={onDelete}
        onRename={onRename}
      />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onOpen}>
          <FolderOpen />
          {t("explorer:contextMenu.open")}
          <ContextMenuShortcut>Enter</ContextMenuShortcut>
        </ContextMenuItem>
        {/* Directory-only: a file has no folder to land the new surface on, so
            both entries stay hidden instead of opening an empty view. Only the
            right-clicked folder is used — the multi-selection does not fan out
            into one tab or window per entry. */}
        {entry.kind === "directory" && (
          <>
            <ContextMenuItem onClick={() => openInNewTab(entry.path)}>
              <PanelsTopLeft />
              {t("explorer:contextMenu.openInNewTab")}
            </ContextMenuItem>
            <ContextMenuItem onClick={() => openInNewWindow(entry.path)}>
              <PictureInPicture2 />
              {t("explorer:contextMenu.openInNewWindow")}
            </ContextMenuItem>
          </>
        )}
        {/* Windows keeps the native SHOpenWithDialog; macOS/Linux fall back
            to the in-app picker, both routed through the explorer view. */}
        <ContextMenuItem disabled={isActionDisabled} onClick={onOpenWith}>
          <AppWindow />
          {t("explorer:contextMenu.openWith")}
        </ContextMenuItem>
        <ContextMenuItem disabled={isActionDisabled || !isSingleSelection} onClick={onTogglePreview}>
          <Eye />
          {t("explorer:contextMenu.preview")}
          <ContextMenuShortcut>{previewBinding}</ContextMenuShortcut>
        </ContextMenuItem>
        {entry.kind === "directory" && (
          <ContextMenuItem
            disabled={isActionDisabled}
            onClick={() => (isFavorited ? removeFavorite(entry.path) : onAddToFavorites())}
          >
            <Star />
            {isFavorited
              ? t("explorer:contextMenu.removeFromFavorites")
              : t("explorer:contextMenu.addToFavorites")}
          </ContextMenuItem>
        )}
        {entry.kind === "directory" && spaces.length > 0 && (
          <ContextMenuSub>
            <ContextMenuSubTrigger disabled={isActionDisabled}>
              <LayoutGrid />
              {t("explorer:contextMenu.addToSpace")}
            </ContextMenuSubTrigger>
            <ContextMenuSubContent>
              {spaces.map((space) => (
                <ContextMenuItem key={space.id} onClick={() => onAddToSpace(space.id)}>
                  <LayoutGrid />
                  {space.name}
                </ContextMenuItem>
              ))}
            </ContextMenuSubContent>
          </ContextMenuSub>
        )}
        {entry.kind === "directory" && (
          <ContextMenuItem
            disabled={isActionDisabled}
            onClick={() => void openTerminalAt(entry.path)}
          >
            <SquareTerminal />
            {t("explorer:contextMenu.openInTerminal")}
          </ContextMenuItem>
        )}
        <ContextMenuItem onClick={() => void copyEntryPath(entry.path)}>
          <ClipboardList />
          {t("explorer:contextMenu.copyPath")}
        </ContextMenuItem>
      </ContextMenuGroup>
      {/* The installed apps' own right-click commands, under the one heading
          dae has always used for them. */}
      <ShellCommandsMenu
        onError={setShellCommandError}
        paths={actionPaths}
        primary={entry.path}
      />
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem disabled={isActionDisabled} onClick={onDuplicate}>
          <Files />
          {t("explorer:contextMenu.duplicate")}
        </ContextMenuItem>
        <ContextMenuSub>
          <ContextMenuSubTrigger disabled={isActionDisabled}>
            <FileArchive />
            {t("explorer:contextMenu.compressAs")}
          </ContextMenuSubTrigger>
          <ContextMenuSubContent>
            {COMPRESS_FORMATS.map(({ encrypted, format, labelKey }) => (
              <ContextMenuItem
                key={labelKey}
                disabled={isActionDisabled}
                onClick={() => onCompress(format, encrypted ?? false)}
              >
                {encrypted ? <LockKeyhole /> : <FileArchive />}
                {t(`explorer:compressFormats.${labelKey}`)}
              </ContextMenuItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
        {isArchiveFile(entry) && (
          <ContextMenuItem disabled={isActionDisabled} onClick={() => onExtract(entry.path)}>
            <PackageOpen />
            {t("explorer:contextMenu.extractHere")}
          </ContextMenuItem>
        )}
        <ContextMenuItem disabled={isActionDisabled} onClick={onMoveTo}>
          <Move />
          {t("explorer:contextMenu.moveTo")}
        </ContextMenuItem>
      </ContextMenuGroup>
      <ContextMenuSeparator />
      <ContextMenuGroup>
        <ContextMenuItem
          disabled={isActionDisabled || !isSingleSelection}
          onClick={() => setPropertiesTarget(entry)}
        >
          <Info />
          {t("explorer:contextMenu.properties")}
          <ContextMenuShortcut>Alt+Enter</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuGroup>
    </>
  );
}

/**
 * The 剪切 / 复制 / 重命名 / 删除 row along the top of the entry menu: four icon
 * buttons with the label under the icon, the way the Windows 11 file menu lays
 * them out.
 *
 * It is there to buy height back. As ordinary menu rows those four actions cost
 * four rows and two separators, and every row the menu grows pushes more of the
 * app-contributed commands below it off the bottom of the screen.
 *
 * The buttons are menu items rather than `<button>`s: arrow keys, Home/End,
 * Enter/Shift+Enter and the menu's own focus styling then keep working, all of
 * which a plain button sitting inside the popup would fall outside of.
 */
function EntryActionRow({
  disabled,
  onCopy,
  onCut,
  onDelete,
  onRename,
}: {
  disabled: boolean;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onRename: () => void;
}) {
  const { t } = useTranslation("explorer");

  const actions = [
    { Icon: Scissors, label: t("explorer:contextMenu.cut"), onSelect: onCut },
    { Icon: Copy, label: t("explorer:contextMenu.copy"), onSelect: onCopy },
    {
      Icon: Pencil,
      // Fixed wording, unlike the row this replaced. The four columns share one
      // row, so letting 批量重命名 appear for a multi-selection would widen all
      // four of them and stretch the whole menu with it.
      label: t("explorer:contextMenu.rename"),
      onSelect: onRename,
    },
    { Icon: Trash2, label: t("explorer:contextMenu.delete"), onSelect: onDelete },
  ];

  return (
    <ContextMenuGroup className="mb-1 grid grid-cols-4 divide-x divide-border/60 overflow-hidden rounded-lg bg-muted/50 p-0.5 ring-1 ring-border/60">
      {actions.map(({ Icon, label, onSelect }) => (
        <ContextMenuItem
          className="h-auto flex-col justify-center gap-1 rounded-md px-1 py-1 text-caption"
          disabled={disabled}
          key={label}
          onClick={onSelect}
        >
          <Icon className="size-[18px]" />
          {/* A single node: the item is a flex column with a gap, so a label
              split across nodes would get a gap between its pieces. */}
          <span className="max-w-full truncate leading-none">{label}</span>
        </ContextMenuItem>
      ))}
    </ContextMenuGroup>
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
