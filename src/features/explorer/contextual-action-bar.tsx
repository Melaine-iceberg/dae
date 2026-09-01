import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  ArchiveTrayIcon,
  ArrowsOutCardinalIcon,
  ClipboardTextIcon,
  CopyIcon,
  DotsThreeIcon,
  FileZipIcon,
  FilesIcon,
  FolderOpenIcon,
  LockKeyIcon,
  PencilIcon,
  ScissorsIcon,
  SquaresFourIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import type { ArchiveFormat } from "@/bindings";
import { localeNumber } from "@/i18n/format";
import { MOD_KEY } from "@/lib/platform";

const COMPRESS_FORMATS: { encrypted?: boolean; format: ArchiveFormat; labelKey: string }[] = [
  { format: "zip", labelKey: "zip" },
  { format: "tar", labelKey: "tar" },
  { format: "tar.gz", labelKey: "tarGz" },
  { format: "7z", labelKey: "7z" },
  { encrypted: true, format: "7z", labelKey: "7zEncrypted" },
];

/**
 * Floating contextual action surface that follows the selection
 * (SKILL.md §19/§20). Appears only while items are selected; destructive
 * actions are visually differentiated.
 */
export function ContextualActionBar({
  archiveSelectionPath,
  hasDirectorySelection,
  isActionDisabled,
  onAddToSpace,
  onClearSelection,
  onCompress,
  onCopy,
  onCopyPaths,
  onCut,
  onDelete,
  onDuplicate,
  onExtract,
  onMoveTo,
  onOpen,
  onRename,
  selectedCount,
}: {
  archiveSelectionPath: string | null;
  hasDirectorySelection: boolean;
  isActionDisabled: boolean;
  onAddToSpace: (spaceId: string) => void;
  onClearSelection: () => void;
  onCompress: (format: ArchiveFormat, encrypted: boolean) => void;
  onCopy: () => void;
  onCopyPaths: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onExtract: (path: string) => void;
  onMoveTo: () => void;
  onOpen: () => void;
  onRename: () => void;
  selectedCount: number;
}) {
  const { t } = useTranslation("explorer");
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  return (
    <div className="absolute bottom-4 left-1/2 z-30 -translate-x-1/2">
      <div
        aria-label={t("explorer:actionBar.ariaLabel")}
        className="animate-float-in flex items-center gap-0.5 rounded-lg bg-popover p-1 shadow-ambient-lg ring-1 ring-border backdrop-blur"
        role="toolbar"
      >
      <span className="shrink-0 px-2.5 text-[13px] text-muted-foreground select-none tabular-nums">
        {t("explorer:actionBar.selectedCount", { number: localeNumber(selectedCount) })}
      </span>
      <div aria-hidden="true" className="mr-0.5 h-5 w-px bg-border" />
      <Button
        aria-label={t("explorer:actionBar.openAria")}
        disabled={isActionDisabled}
        onClick={onOpen}
        size="icon"
        title={t("explorer:actionBar.openTitle")}
        type="button"
        variant="ghost"
      >
        <FolderOpenIcon />
      </Button>
      <Button
        aria-label={t("explorer:actionBar.copyAria")}
        disabled={isActionDisabled}
        onClick={onCopy}
        size="icon"
        title={t("explorer:actionBar.copyTitle", { modifier: MOD_KEY })}
        type="button"
        variant="ghost"
      >
        <CopyIcon />
      </Button>
      <Button
        aria-label={t("explorer:actionBar.cutAria")}
        disabled={isActionDisabled}
        onClick={onCut}
        size="icon"
        title={t("explorer:actionBar.cutTitle", { modifier: MOD_KEY })}
        type="button"
        variant="ghost"
      >
        <ScissorsIcon />
      </Button>
      <Button
        aria-label={t("explorer:actionBar.renameAria")}
        disabled={isActionDisabled}
        onClick={onRename}
        size="icon"
        title={t("explorer:actionBar.renameTitle")}
        type="button"
        variant="ghost"
      >
        <PencilIcon />
      </Button>
      {archiveSelectionPath && (
        <Button
          aria-label={t("explorer:actionBar.extractAria")}
          disabled={isActionDisabled}
          onClick={() => onExtract(archiveSelectionPath)}
          size="icon"
          title={t("explorer:actionBar.extractTitle")}
          type="button"
          variant="ghost"
        >
          <ArchiveTrayIcon />
        </Button>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={t("explorer:actionBar.moreAria")}
          className="flex size-8 items-center justify-center rounded-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
          disabled={isActionDisabled}
          title={t("explorer:actionBar.moreTitle")}
        >
          <DotsThreeIcon className="size-4" weight="bold" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center" side="top">
          <DropdownMenuItem disabled={isActionDisabled} onClick={onDuplicate}>
            <FilesIcon />
            {t("explorer:contextMenu.duplicate")}
          </DropdownMenuItem>
          <DropdownMenuSub>
            <DropdownMenuSubTrigger disabled={isActionDisabled}>
              <FileZipIcon />
              {t("explorer:contextMenu.compressAs")}
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              {COMPRESS_FORMATS.map(({ encrypted, format, labelKey }) => (
                <DropdownMenuItem
                  key={labelKey}
                  disabled={isActionDisabled}
                  onClick={() => onCompress(format, encrypted ?? false)}
                >
                  {encrypted ? <LockKeyIcon /> : <FileZipIcon />}
                  {t(`explorer:compressFormats.${labelKey}`)}
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
          <DropdownMenuItem disabled={isActionDisabled} onClick={onMoveTo}>
            <ArrowsOutCardinalIcon />
            {t("explorer:contextMenu.moveTo")}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onCopyPaths}>
            <ClipboardTextIcon />
            {t("explorer:contextMenu.copyPath")}
          </DropdownMenuItem>
          {hasDirectorySelection &&
            spaces.slice(0, 3).map((space) => (
              <DropdownMenuItem key={space.id} onClick={() => onAddToSpace(space.id)}>
                <SquaresFourIcon />
                {t("explorer:contextMenu.addToSpaceNamed", { name: space.name })}
              </DropdownMenuItem>
            ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        aria-label={t("explorer:actionBar.deleteAria")}
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
        disabled={isActionDisabled}
        onClick={onDelete}
        size="icon"
        title={t("explorer:actionBar.deleteTitle")}
        type="button"
        variant="ghost"
      >
        <TrashIcon />
      </Button>
      <div aria-hidden="true" className="ml-0.5 h-5 w-px bg-border" />
      <Button
        aria-label={t("explorer:actionBar.clearAria")}
        onClick={onClearSelection}
        size="icon"
        title={t("explorer:actionBar.clearTitle")}
        type="button"
        variant="ghost"
      >
        <XIcon />
      </Button>
      </div>
    </div>
  );
}
