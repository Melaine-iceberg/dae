import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ClipboardTextIcon,
  FolderOpenIcon,
  StarIcon,
  TabsIcon,
  XIcon,
} from "@phosphor-icons/react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { openInNewTabAtom } from "@/features/explorer/tabs";
import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  removeFavoriteAtom,
} from "@/features/sidebar/sidebar-atoms";

import { navigateToFolderAtom } from "./workspace-atoms";
import { LocationCard, WorkspacePage, WorkspacePageHeader } from "./workspace-components";

/** The Favorites surface: every favorited folder as an expressive card. */
export function FavoritesView() {
  const favorites = useAtomValue(favoritesAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const removeFavorite = useSetAtom(removeFavoriteAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);

  useEffect(() => {
    void ensureFavoritesLoaded();
  }, [ensureFavoritesLoaded]);

  return (
    <WorkspacePage aria-label="收藏">
      <WorkspacePageHeader title="收藏" description="常用文件夹，一键直达。" />

      {favorites === null ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton className="h-[62px] rounded-xl" key={index} />
          ))}
        </div>
      ) : favorites.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <StarIcon />
            </EmptyMedia>
            <EmptyTitle>暂无收藏</EmptyTitle>
            <EmptyDescription>
              在文件夹中点击工具栏的星标，或将文件夹拖到侧边栏的“收藏”，即可把常用位置放在这里。
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {favorites.map((favorite) => (
            <ContextMenu key={favorite.path}>
              <ContextMenuTrigger>
                <LocationCard
                  description={favorite.path}
                  icon={StarIcon}
                  iconClassName="fill-amber-400 text-amber-500"
                  onClick={() => navigateToFolder(favorite.path)}
                  title={favorite.name}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => navigateToFolder(favorite.path)}>
                    <FolderOpenIcon />
                    打开
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewTab(favorite.path)}>
                    <TabsIcon />
                    在新标签页打开
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void copyPath(favorite.path)}>
                    <ClipboardTextIcon />
                    复制文件地址
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => removeFavorite(favorite.path)}>
                    <XIcon />
                    从收藏移除
                  </ContextMenuItem>
                </ContextMenuGroup>
              </ContextMenuContent>
            </ContextMenu>
          ))}
        </div>
      )}
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
