import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ClipboardList, FolderOpen, PictureInPicture2, Star, PanelsTopLeft, X } from "lucide-react";

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
import { ErrorState } from "@/components/ui/error-state";
import { openInNewTabAtom, openPathInNewWindowAtom } from "@/features/explorer/tabs";
import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  favoritesErrorAtom,
  removeFavoriteAtom,
} from "@/features/sidebar/sidebar-atoms";

import { navigateToFolderAtom } from "./workspace-atoms";
import { LocationCard, WorkspacePage, WorkspacePageHeader } from "./workspace-components";

/** The Favorites surface: every favorited folder as an expressive card. */
export function FavoritesView() {
  const { t } = useTranslation("workspace");
  const favorites = useAtomValue(favoritesAtom);
  const favoritesError = useAtomValue(favoritesErrorAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const removeFavorite = useSetAtom(removeFavoriteAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);
  const openInNewWindow = useSetAtom(openPathInNewWindowAtom);

  useEffect(() => {
    void ensureFavoritesLoaded();
  }, [ensureFavoritesLoaded]);

  return (
    <WorkspacePage aria-label={t("favorites.title")}>
      {/* No subtitle: "常用文件夹，一键直达。" is a tagline, not information. */}
      <WorkspacePageHeader title={t("favorites.title")} />

      {favoritesError !== null ? (
        // A failed read is not an empty list: say so, and offer the one action
        // that can change the outcome.
        <ErrorState
          className="min-h-64"
          description={favoritesError}
          onRetry={() => void ensureFavoritesLoaded()}
          title={t("loadError.favoritesTitle")}
        />
      ) : favorites === null ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton className="h-location-card rounded-md" key={index} />
          ))}
        </div>
      ) : favorites.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Star />
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
                  icon={Star}
                  iconClassName="fill-current text-primary"
                  onClick={() => navigateToFolder(favorite.path)}
                  title={favorite.name}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => navigateToFolder(favorite.path)}>
                    <FolderOpen />
                    {t("favorites.open")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewTab(favorite.path)}>
                    <PanelsTopLeft />
                    {t("favorites.openInNewTab")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => openInNewWindow(favorite.path)}>
                    <PictureInPicture2 />
                    {t("favorites.openInNewWindow")}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void copyPath(favorite.path)}>
                    <ClipboardList />
                    {t("favorites.copyPath")}
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem onClick={() => removeFavorite(favorite.path)}>
                    <X />
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
