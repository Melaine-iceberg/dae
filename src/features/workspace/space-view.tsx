import { useEffect, useState, type FormEvent } from "react";
import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { atom } from "jotai";
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

import {
  deleteSpace,
  ensureSpacesLoadedAtom,
  removeSpaceItem,
  renameSpace,
  spacesAtom,
} from "./spaces-atoms";
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
      <WorkspacePage aria-label="空间">
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <SquaresFourIcon />
            </EmptyMedia>
            <EmptyTitle>空间不存在</EmptyTitle>
            <EmptyDescription>它可能已被删除。</EmptyDescription>
          </EmptyHeader>
          <Button onClick={() => openSurface({ kind: "overview" })} size="sm" type="button">
            返回概览
          </Button>
        </Empty>
      </WorkspacePage>
    );
  }

  return (
    <WorkspacePage aria-label={space ? `空间 ${space.name}` : "空间"}>
      {space === null ? (
        <Skeleton className="h-9 w-40 rounded-lg" />
      ) : (
        <WorkspacePageHeader
          actions={
            <>
              <Button
                aria-label="重命名空间"
                onClick={() => {
                  setRenameValue(space.name);
                  setRenaming(true);
                }}
                size="icon"
                title="重命名空间"
                type="button"
                variant="ghost"
              >
                <PencilSimpleIcon />
              </Button>
              {!space.isPreset && (
                <Button
                  aria-label="删除空间"
                  className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                  onClick={() => setConfirmingDelete(true)}
                  size="icon"
                  title="删除空间"
                  type="button"
                  variant="ghost"
                >
                  <TrashIcon />
                </Button>
              )}
            </>
          }
          title={space.name}
          description={
            space.items.length > 0 ? `${space.items.length} 个位置` : "把相关的文件夹聚集到这里。"
          }
        />
      )}

      {renaming && space && (
        <form className="flex max-w-sm items-center gap-2" onSubmit={submitRename}>
          <Input
            aria-label="空间名称"
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
          <Button aria-label="确认重命名" size="icon" title="确认" type="submit" variant="outline">
            <CheckIcon />
          </Button>
          <Button
            aria-label="取消重命名"
            onClick={() => setRenaming(false)}
            size="icon"
            title="取消"
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
            <EmptyTitle>空间还是空的</EmptyTitle>
            <EmptyDescription>
              把文件夹拖到侧边栏中的“{space.name}”，或在文件夹上右键选择“添加到空间”。
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
                    打开
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewTab(item.path)}>
                    <TabsIcon />
                    在新标签页打开
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void copyPath(item.path)}>
                    <ClipboardTextIcon />
                    复制文件地址
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => void removeSpaceItem(space.id, item.path)}>
                    <XIcon />
                    从空间中移除
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
            <DialogTitle>删除空间“{space?.name}”？</DialogTitle>
            <DialogDescription>
              只会删除这个空间的组织方式，里面的文件夹和文件不会被删除。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              disabled={isDeleting}
              onClick={() => setConfirmingDelete(false)}
              type="button"
              variant="outline"
            >
              取消
            </Button>
            <Button
              disabled={isDeleting}
              onClick={() => void confirmDelete()}
              type="button"
              variant="destructive"
            >
              删除空间
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
