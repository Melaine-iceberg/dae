import { useEffect, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ArrowRightIcon,
  ClockCounterClockwiseIcon,
  FolderIcon,
  SquaresFourIcon,
  StarIcon,
} from "@phosphor-icons/react";

import { commands, type SystemPlace } from "@/bindings";
import { cn } from "@/lib/utils";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
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
import {
  DIRECTORY_PRESENTATION,
  getFilePresentation,
  getPresentationIconClassName,
} from "@/features/explorer/file-icons";
import { PLACE_PRESENTATION } from "@/features/sidebar/place-presentation";
import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  hiddenPlacesAtom,
} from "@/features/sidebar/sidebar-atoms";

import { ensureRecentsLoadedAtom, recentsAtom, recordRecentItem } from "./recents-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "./spaces-atoms";
import { getSpaceAccent } from "./space-identity";
import { getSpaceDisplayName } from "./types";
import { ProjectsSection } from "./projects-section";
import { navigateToFolderAtom, openSurfaceAtom } from "./workspace-atoms";
import {
  LocationCard,
  SectionHeader,
  WorkspacePage,
  WorkspacePageHeader,
  formatRecentTime,
} from "./workspace-components";

const RECENTS_PREVIEW_COUNT = 6;
const FAVORITES_PREVIEW_COUNT = 6;

/**
 * Overview is the default landing surface: quick access to common locations,
 * recent activity, favorites, and the user's spaces — not a raw filesystem
 * path.
 */
export function OverviewView() {
  const { t } = useTranslation("workspace");
  const [places, setPlaces] = useState<SystemPlace[] | null>(null);
  const hiddenPlaces = useAtomValue(hiddenPlacesAtom);
  const setHiddenPlaces = useSetAtom(hiddenPlacesAtom);
  const recents = useAtomValue(recentsAtom);
  const favorites = useAtomValue(favoritesAtom) ?? [];
  const spaces = useAtomValue(spacesAtom);
  const ensureRecentsLoaded = useSetAtom(ensureRecentsLoadedAtom);
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const openSurface = useSetAtom(openSurfaceAtom);

  useEffect(() => {
    void commands
      .getSystemPlaces()
      .then(setPlaces)
      .catch((error: unknown) => console.warn("Unable to load system places", error));
    void ensureRecentsLoaded();
    void ensureFavoritesLoaded();
    void ensureSpacesLoaded();
  }, [ensureRecentsLoaded, ensureFavoritesLoaded, ensureSpacesLoaded]);

  const visiblePlaces = (places ?? []).filter((place) => !hiddenPlaces.includes(place.kind));
  const recentPreview = (recents ?? []).slice(0, RECENTS_PREVIEW_COUNT);
  const favoritePreview = favorites.slice(0, FAVORITES_PREVIEW_COUNT);

  const openRecent = (path: string, kind: "directory" | "file") => {
    if (kind === "directory") {
      navigateToFolder(path);
      return;
    }

    recordRecentItem(path, "file", "opened");
    void openPath(path).catch((error: unknown) => console.warn(`Unable to open ${path}`, error));
  };

  return (
    <WorkspacePage aria-label={t("overview.title")}>
      <WorkspacePageHeader title={t("overview.title")} description={t("overview.description")} />

      <ProjectsSection recents={recents} />

      <section aria-label={t("overview.quickAccess")}>
        <SectionHeader title={t("overview.quickAccess")} />
        {places === null ? (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
            {Array.from({ length: 4 }, (_, index) => (
              <Skeleton className="h-[62px] rounded-xl" key={index} />
            ))}
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
            {visiblePlaces.map((place) => {
              const presentation = PLACE_PRESENTATION[place.kind];
              return (
                <ContextMenu key={place.kind}>
                  <ContextMenuTrigger>
                    <LocationCard
                      description={place.path}
                      icon={presentation.icon}
                      iconClassName="text-muted-foreground"
                      onClick={() => navigateToFolder(place.path)}
                      title={presentation.label}
                    />
                  </ContextMenuTrigger>
                  <ContextMenuContent>
                    <ContextMenuItem onClick={() => navigateToFolder(place.path)}>
                      <FolderIcon />
                      {t("overview.open")}
                    </ContextMenuItem>
                    <ContextMenuItem onClick={() => setHiddenPlaces([...hiddenPlaces, place.kind])}>
                      <StarIcon />
                      {t("overview.hideFromQuickAccess")}
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              );
            })}
          </div>
        )}
      </section>

      <section aria-label={t("overview.recentsTitle")}>
        <SectionHeader
          action={
            (recents?.length ?? 0) > RECENTS_PREVIEW_COUNT && (
              <button
                className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
                onClick={() => openSurface({ kind: "recents" })}
                type="button"
              >
                {t("overview.viewAll")}
                <ArrowRightIcon className="size-3" />
              </button>
            )
          }
          title={t("overview.recentsTitle")}
        />
        {recents === null ? (
          <div className="flex flex-col gap-1">
            {Array.from({ length: 3 }, (_, index) => (
              <Skeleton className="h-9 rounded-lg" key={index} />
            ))}
          </div>
        ) : recentPreview.length === 0 ? (
          <Empty className="border-none py-6">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <ClockCounterClockwiseIcon />
              </EmptyMedia>
              <EmptyTitle className="text-sm">{t("overview.recentsEmptyTitle")}</EmptyTitle>
              <EmptyDescription className="text-xs">
                {t("overview.recentsEmptyDescription")}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <ul className="flex flex-col">
            {recentPreview.map((item) => {
              const presentation =
                item.kind === "directory" ? DIRECTORY_PRESENTATION : getFilePresentation(item.name);
              const EntryIcon = presentation.icon;
              return (
                <li key={item.path}>
                  <button
                    className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    onClick={() =>
                      openRecent(item.path, item.kind === "directory" ? "directory" : "file")
                    }
                    title={item.path}
                    type="button"
                  >
                    <EntryIcon
                      className={cn("size-4 shrink-0", getPresentationIconClassName(presentation))}
                      weight={item.kind === "directory" ? "fill" : undefined}
                    />
                    <span className="min-w-0 flex-1 truncate text-[13px]">{item.name}</span>
                    <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                      {formatRecentTime(item.accessedAt, t("recents.groups.yesterday"))}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section aria-label={t("overview.favoritesTitle")}>
        <SectionHeader
          action={
            favorites.length > FAVORITES_PREVIEW_COUNT && (
              <button
                className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
                onClick={() => openSurface({ kind: "favorites" })}
                type="button"
              >
                {t("overview.viewAll")}
                <ArrowRightIcon className="size-3" />
              </button>
            )
          }
          title={t("overview.favoritesTitle")}
        />
        {favoritePreview.length === 0 ? (
          <Empty className="border-none py-6">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <StarIcon />
              </EmptyMedia>
              <EmptyTitle className="text-sm">{t("overview.favoritesEmptyTitle")}</EmptyTitle>
              <EmptyDescription className="text-xs">
                {t("overview.favoritesEmptyDescription")}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            {favoritePreview.map((favorite) => (
              <LocationCard
                description={favorite.path}
                icon={StarIcon}
                iconClassName="fill-amber-400 text-amber-500"
                key={favorite.path}
                onClick={() => navigateToFolder(favorite.path)}
                title={favorite.name}
              />
            ))}
          </div>
        )}
      </section>

      <section aria-label={t("overview.spacesTitle")}>
        <SectionHeader title={t("overview.spacesTitle")} />
        {spaces === null ? (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            {Array.from({ length: 4 }, (_, index) => (
              <Skeleton className="h-[62px] rounded-xl" key={index} />
            ))}
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            {spaces.map((space) => (
              <LocationCard
                description={
                  space.items.length > 0
                    ? t("spaces.itemCount", { count: space.items.length })
                    : t("spaces.emptyLabel")
                }
                icon={SquaresFourIcon}
                iconClassName={getSpaceAccent(space.id).text}
                key={space.id}
                onClick={() => openSurface({ kind: "space", spaceId: space.id })}
                title={getSpaceDisplayName(space)}
              />
            ))}
          </div>
        )}
      </section>
    </WorkspacePage>
  );
}
