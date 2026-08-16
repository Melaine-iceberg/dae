import { useEffect, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  ArrowRightIcon,
  ClockCounterClockwiseIcon,
  FileTextIcon,
  FolderIcon,
  SquaresFourIcon,
  StarIcon,
} from "@phosphor-icons/react";

import { commands, type SystemPlace } from "@/bindings";

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
import { PLACE_PRESENTATION } from "@/features/sidebar/place-presentation";
import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  hiddenPlacesAtom,
} from "@/features/sidebar/sidebar-atoms";

import { ensureRecentsLoadedAtom, recentsAtom, recordRecentItem } from "./recents-atoms";
import { ensureSpacesLoadedAtom, spacesAtom } from "./spaces-atoms";
import { getSpaceAccent } from "./space-identity";
import { navigateToFolderAtom, openSurfaceAtom } from "./workspace-atoms";
import {
  LocationCard,
  SectionHeader,
  WorkspacePage,
  WorkspacePageHeader,
} from "./workspace-components";

const RECENTS_PREVIEW_COUNT = 6;
const FAVORITES_PREVIEW_COUNT = 6;

/**
 * Overview is the default landing surface: quick access to common locations,
 * recent activity, favorites, and the user's spaces — not a raw filesystem
 * path.
 */
export function OverviewView() {
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
    <WorkspacePage aria-label="概览">
      <WorkspacePageHeader title="概览" description="快速回到你的位置、空间和最近的工作。" />

      <section aria-label="快捷入口">
        <SectionHeader title="快捷入口" />
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
                      打开
                    </ContextMenuItem>
                    <ContextMenuItem onClick={() => setHiddenPlaces([...hiddenPlaces, place.kind])}>
                      <StarIcon />
                      从快捷入口隐藏
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              );
            })}
          </div>
        )}
      </section>

      <section aria-label="最近使用">
        <SectionHeader
          action={
            (recents?.length ?? 0) > RECENTS_PREVIEW_COUNT && (
              <button
                className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
                onClick={() => openSurface({ kind: "recents" })}
                type="button"
              >
                查看全部
                <ArrowRightIcon className="size-3" />
              </button>
            )
          }
          title="最近使用"
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
              <EmptyTitle className="text-sm">暂无最近使用</EmptyTitle>
              <EmptyDescription className="text-xs">
                你浏览的文件夹和打开的文件会显示在这里。
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <ul className="flex flex-col">
            {recentPreview.map((item) => (
              <li key={item.path}>
                <button
                  className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  onClick={() =>
                    openRecent(item.path, item.kind === "directory" ? "directory" : "file")
                  }
                  title={item.path}
                  type="button"
                >
                  <FolderIcon
                    className={item.kind === "directory" ? "size-4 shrink-0 text-folder" : "hidden"}
                    weight="fill"
                  />
                  <FileTextIcon
                    className={
                      item.kind === "directory" ? "hidden" : "size-4 shrink-0 text-muted-foreground"
                    }
                  />
                  <span className="min-w-0 flex-1 truncate text-[13px]">{item.name}</span>
                  <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                    {formatRecentTime(item.accessedAt)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section aria-label="收藏">
        <SectionHeader
          action={
            favorites.length > FAVORITES_PREVIEW_COUNT && (
              <button
                className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
                onClick={() => openSurface({ kind: "favorites" })}
                type="button"
              >
                查看全部
                <ArrowRightIcon className="size-3" />
              </button>
            )
          }
          title="收藏"
        />
        {favoritePreview.length === 0 ? (
          <Empty className="border-none py-6">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <StarIcon />
              </EmptyMedia>
              <EmptyTitle className="text-sm">暂无收藏</EmptyTitle>
              <EmptyDescription className="text-xs">
                在文件夹中点击工具栏的星标，或将文件夹拖到侧边栏的收藏，即可添加。
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

      <section aria-label="空间">
        <SectionHeader title="空间" />
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
                description={space.items.length > 0 ? `${space.items.length} 个位置` : "空空间"}
                icon={SquaresFourIcon}
                iconClassName={getSpaceAccent(space.id).text}
                key={space.id}
                onClick={() => openSurface({ kind: "space", spaceId: space.id })}
                title={space.name}
              />
            ))}
          </div>
        )}
      </section>
    </WorkspacePage>
  );
}

function formatRecentTime(accessedAt: number): string {
  const date = new Date(accessedAt);
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfDate = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();

  if (startOfDate === startOfToday) {
    return date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
  }

  if (startOfToday - startOfDate === 86_400_000) {
    return "昨天";
  }

  return date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}
