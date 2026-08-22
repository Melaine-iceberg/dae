import { useEffect, useState, type FormEvent } from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { atom } from "jotai";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  CheckIcon,
  ClipboardTextIcon,
  FolderIcon,
  FolderOpenIcon,
  PencilSimpleIcon,
  SquaresFourIcon,
  TabsIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { openInNewTabAtom } from "@/features/explorer/tabs";
import { cn } from "@/lib/utils";

import {
  deleteSpace,
  ensureSpacesLoadedAtom,
  removeSpaceItem,
  renameSpace,
  spacesAtom,
} from "./spaces-atoms";
import { getSpaceAccent } from "./space-identity";
import { getSpaceDisplayName } from "./types";
import { navigateToFolderAtom, openSurfaceAtom } from "./workspace-atoms";
import { LocationCard, WorkspacePage, WorkspacePageHeader } from "./workspace-components";

/**
 * Set by the sidebar's "重命名空间" action so the space view opens directly
 * in rename mode. Holds a space id, or null when idle.
 */
export const spaceRenameRequestAtom = atom<string | null>(null);

/**
 * One Space: a contextual workspace of pinned folders and locations.
 * Items are pointers — removing them here never touches the filesystem.
 */
export function SpaceView({ spaceId }: { spaceId: string }) {
  const { t } = useTranslation("workspace");
  const spaces = useAtomValue(spacesAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);
  const openSurface = useSetAtom(openSurfaceAtom);
  const [renameRequest, setRenameRequest] = useAtom(spaceRenameRequestAtom);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  const space = spaces?.find((candidate) => candidate.id === spaceId) ?? null;

  useEffect(() => {
    if (renameRequest === spaceId && space) {
      setRenameValue(space.name);
      setRenaming(true);
      setRenameRequest(null);
    }
  }, [renameRequest, space, spaceId, setRenameRequest]);

  const submitRename = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const nextName = renameValue.trim();
    setRenaming(false);
    if (!space || !nextName || nextName === space.name) return;
    void renameSpace(space.id, nextName);
  };

  const confirmDelete = async () => {
    if (!space) return;
    setIsDeleting(true);
    const deleted = await deleteSpace(space.id);
    setIsDeleting(false);
    setConfirmingDelete(false);
    if (deleted) {
      openSurface({ kind: "overview" });
    }
  };

  if (spaces !== null && space === null) {
    return (
      <WorkspacePage aria-label={t("space.ariaLabel")}>
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <SquaresFourIcon />
            </EmptyMedia>
            <EmptyTitle>{t("space.notFoundTitle")}</EmptyTitle>
            <EmptyDescription>{t("space.notFoundDescription")}</EmptyDescription>
          </EmptyHeader>
          <Button onClick={() => openSurface({ kind: "overview" })} size="sm" type="button">
            {t("space.backToOverview")}
          </Button>
        </Empty>
      </WorkspacePage>
    );
  }

  return (
    <WorkspacePage
      aria-label={
        space
          ? t("space.ariaLabelWithName", { name: getSpaceDisplayName(space) })
          : t("space.ariaLabel")
      }
    >
      {space === null ? (
        <Skeleton className="h-9 w-40 rounded-lg" />
      ) : (
        <WorkspacePageHeader
          actions={
            <>
              <Button
                aria-label={t("space.rename")}
                onClick={() => {
                  setRenameValue(space.name);
                  setRenaming(true);
                }}
                size="icon"
                title={t("space.rename")}
                type="button"
                variant="ghost"
              >
                <PencilSimpleIcon />
              </Button>
              {!space.isPreset && (
                <Button
                  aria-label={t("space.delete")}
                  className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                  onClick={() => setConfirmingDelete(true)}
                  size="icon"
                  title={t("space.delete")}
                  type="button"
                  variant="ghost"
                >
                  <TrashIcon />
                </Button>
              )}
            </>
          }
          icon={
            <span
              aria-hidden="true"
              className={cn(
                "flex size-11 shrink-0 items-center justify-center rounded-xl",
                getSpaceAccent(space.id).tile,
              )}
            >
              <SquaresFourIcon className={cn("size-5.5", getSpaceAccent(space.id).text)} />
            </span>
          }
          title={getSpaceDisplayName(space)}
          description={
            space.items.length > 0
              ? t("spaces.itemCount", { count: space.items.length })
              : t("spaces.emptyHint")
          }
        />
      )}

      {renaming && space && (
        <form className="flex max-w-sm items-center gap-2" onSubmit={submitRename}>
          <Input
            aria-label={t("space.nameAriaLabel")}
            autoFocus
            onChange={(event) => setRenameValue(event.target.value)}
            onFocus={(event) => event.currentTarget.select()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                setRenaming(false);
              }
            }}
            value={renameValue}
          />
          <Button
            aria-label={t("space.confirmRename")}
            size="icon"
            title={t("space.confirm")}
            type="submit"
            variant="outline"
          >
            <CheckIcon />
          </Button>
          <Button
            aria-label={t("space.cancelRename")}
            onClick={() => setRenaming(false)}
            size="icon"
            title={t("space.cancel")}
            type="button"
            variant="ghost"
          >
            <XIcon />
          </Button>
        </form>
      )}

      {space === null ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton className="h-[62px] rounded-xl" key={index} />
          ))}
        </div>
      ) : space.items.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <SquaresFourIcon />
            </EmptyMedia>
            <EmptyTitle>{t("space.emptyTitle")}</EmptyTitle>
            <EmptyDescription>
              {t("space.emptyDescription", { name: getSpaceDisplayName(space) })}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {space.items.map((item) => (
            <ContextMenu key={item.path}>
              <ContextMenuTrigger>
                <LocationCard
                  description={item.path}
                  icon={FolderIcon}
                  iconClassName="text-folder"
                  onClick={() => navigateToFolder(item.path)}
                  title={item.name}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => navigateToFolder(item.path)}>
                    <FolderOpenIcon />
                    {t("space.open")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewTab(item.path)}>
                    <TabsIcon />
                    {t("space.openInNewTab")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void copyPath(item.path)}>
                    <ClipboardTextIcon />
                    {t("space.copyPath")}
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => void removeSpaceItem(space.id, item.path)}>
                    <XIcon />
                    {t("space.remove")}
                  </ContextMenuItem>
                </ContextMenuGroup>
              </ContextMenuContent>
            </ContextMenu>
          ))}
        </div>
      )}

      <Dialog onOpenChange={setConfirmingDelete} open={confirmingDelete}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("space.deleteTitle", { name: space ? getSpaceDisplayName(space) : "" })}
            </DialogTitle>
            <DialogDescription>{t("space.deleteDescription")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              disabled={isDeleting}
              onClick={() => setConfirmingDelete(false)}
              type="button"
              variant="outline"
            >
              {t("space.cancel")}
            </Button>
            <Button
              disabled={isDeleting}
              onClick={() => void confirmDelete()}
              type="button"
              variant="destructive"
            >
              {t("space.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </WorkspacePage>
  );
}

async function copyPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
  }
}
