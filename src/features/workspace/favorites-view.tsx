import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation("workspace");
  const favorites = useAtomValue(favoritesAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const removeFavorite = useSetAtom(removeFavoriteAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);

  useEffect(() => {
    void ensureFavoritesLoaded();
  }, [ensureFavoritesLoaded]);

  return (
    <WorkspacePage aria-label={t("favorites.title")}>
      <WorkspacePageHeader
        title={t("favorites.title")}
        description={t("favorites.description")}
      />

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
            <EmptyTitle>{t("favorites.emptyTitle")}</EmptyTitle>
            <EmptyDescription>{t("favorites.emptyDescription")}</EmptyDescription>
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
                  iconClassName="fill-white text-white"
                  onClick={() => navigateToFolder(favorite.path)}
                  tileClassName="tile-folder"
                  title={favorite.name}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => navigateToFolder(favorite.path)}>
                    <FolderOpenIcon />
                    {t("favorites.open")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewTab(favorite.path)}>
                    <TabsIcon />
                    {t("favorites.openInNewTab")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void copyPath(favorite.path)}>
                    <ClipboardTextIcon />
                    {t("favorites.copyPath")}
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => removeFavorite(favorite.path)}>
                    <XIcon />
                    {t("favorites.remove")}
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
