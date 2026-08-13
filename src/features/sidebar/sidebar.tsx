import { useCallback, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  DownloadIcon,
  HardDriveIcon,
  HouseIcon,
  ImageIcon,
  MonitorIcon,
  MusicIcon,
  NotebookTextIcon,
  StarIcon,
  Trash2Icon,
  UsbIcon,
  VideoIcon,
  type LucideIcon,
} from "lucide-react";

import { commands } from "@/bindings";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { cn, formatBytes } from "@/lib/utils";

import { activeTabIdAtom, getTabNavigator } from "@/features/explorer/tabs";

import {
  ensureFavoritesLoadedAtom,
  favoritesAtom,
  removeFavoriteAtom,
  reorderFavoritesAtom,
  sidebarVisibleAtom,
} from "./sidebar-atoms";
import type { DiskVolume, Favorite, PlaceKind, SystemPlace } from "./types";
import { useDiskVolumes } from "./use-disk-volumes";

const PLACE_PRESENTATION: Record<PlaceKind, { icon: LucideIcon; label: string }> = {
  home: { icon: HouseIcon, label: "主文件夹" },
  desktop: { icon: MonitorIcon, label: "桌面" },
  documents: { icon: NotebookTextIcon, label: "文档" },
  downloads: { icon: DownloadIcon, label: "下载" },
  pictures: { icon: ImageIcon, label: "图片" },
  music: { icon: MusicIcon, label: "音乐" },
  videos: { icon: VideoIcon, label: "视频" },
};

export function Sidebar() {
  const visible = useAtomValue(sidebarVisibleAtom);
  if (!visible) return null;

  return <SidebarContent />;
}

function SidebarContent() {
  const [places, setPlaces] = useState<SystemPlace[]>([]);
  const favorites = useAtomValue(favoritesAtom) ?? [];
  const ensureFavoritesLoaded = useSetAtom(ensureFavoritesLoadedAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const navigator = getTabNavigator(activeTabId);
  const { directory } = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const currentPath = directory?.path ?? null;
  const volumes = useDiskVolumes(currentPath);

  useEffect(() => {
    void commands
      .getSystemPlaces()
      .then(setPlaces)
      .catch((error: unknown) => console.warn("Unable to load system places", error));
    void ensureFavoritesLoaded();
  }, [ensureFavoritesLoaded]);

  const navigateTo = useCallback((path: string) => void navigator.navigate(path), [navigator]);

  return (
    <nav aria-label="侧边栏" className="flex w-60 shrink-0 flex-col border-r bg-muted/40">
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-3">
        <SidebarSection title="常用位置">
          {places.map((place) => {
            const presentation = PLACE_PRESENTATION[place.kind];
            return (
              <SidebarItem
                icon={presentation.icon}
                isActive={currentPath === place.path}
                key={place.kind}
                label={presentation.label}
                onClick={() => navigateTo(place.path)}
                title={place.path}
              />
            );
          })}
        </SidebarSection>

        <FavoritesSection currentPath={currentPath} favorites={favorites} onNavigate={navigateTo} />

        <SidebarSection title="设备">
          {volumes.map((volume) => (
            <DiskItem
              isActive={currentPath === volume.mountPoint}
              key={volume.mountPoint}
              onNavigate={navigateTo}
              volume={volume}
            />
          ))}
        </SidebarSection>
      </div>
    </nav>
  );
}

function FavoritesSection({
  currentPath,
  favorites,
  onNavigate,
}: {
  currentPath: string | null;
  favorites: Favorite[];
  onNavigate: (path: string) => void;
}) {
  const removeFavorite = useSetAtom(removeFavoriteAtom);
  const reorderFavorites = useSetAtom(reorderFavoritesAtom);
  const [draggedPath, setDraggedPath] = useState<string | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);

  const resetDrag = () => {
    setDraggedPath(null);
    setDropIndex(null);
  };

  const handleDrop = () => {
    if (draggedPath === null || dropIndex === null) {
      resetDrag();
      return;
    }

    const fromIndex = favorites.findIndex((favorite) => favorite.path === draggedPath);
    if (fromIndex === -1) {
      resetDrag();
      return;
    }

    const reordered = [...favorites];
    const [moved] = reordered.splice(fromIndex, 1);
    reordered.splice(dropIndex > fromIndex ? dropIndex - 1 : dropIndex, 0, moved);
    reorderFavorites(reordered);
    resetDrag();
  };

  return (
    <SidebarSection title="收藏夹">
      <div
        data-sidebar-favorites-drop-target=""
        onDragOver={(event) => {
          if (draggedPath !== null) {
            event.preventDefault();
          }
        }}
        onDrop={(event) => {
          if (draggedPath !== null) {
            event.preventDefault();
            handleDrop();
          }
        }}
      >
        {favorites.length === 0 && (
          <p className="px-2 py-1 text-xs text-muted-foreground">
            将文件夹拖到此处，或点击工具栏星标收藏当前目录
          </p>
        )}
        {favorites.map((favorite, index) => (
          <div key={favorite.path}>
            {dropIndex === index && draggedPath !== null && <DropIndicator />}
            <ContextMenu>
              <ContextMenuTrigger
                draggable
                onDragEnd={resetDrag}
                onDragOver={(event) => {
                  if (draggedPath === null) return;
                  event.preventDefault();
                  const rect = event.currentTarget.getBoundingClientRect();
                  setDropIndex(event.clientY < rect.top + rect.height / 2 ? index : index + 1);
                }}
                onDragStart={(event) => {
                  setDraggedPath(favorite.path);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", favorite.path);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  handleDrop();
                }}
              >
                <SidebarItem
                  icon={StarIcon}
                  isActive={currentPath === favorite.path}
                  label={favorite.name}
                  onClick={() => onNavigate(favorite.path)}
                  title={favorite.path}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem onClick={() => removeFavorite(favorite.path)}>
                  <Trash2Icon />
                  从收藏移除
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          </div>
        ))}
        {dropIndex === favorites.length && draggedPath !== null && <DropIndicator />}
      </div>
    </SidebarSection>
  );
}

function DropIndicator() {
  return <div aria-hidden="true" className="mx-2 h-0.5 rounded-full bg-primary" />;
}

function DiskItem({
  isActive,
  onNavigate,
  volume,
}: {
  isActive: boolean;
  onNavigate: (path: string) => void;
  volume: DiskVolume;
}) {
  const presentation = getDiskPresentation(volume);
  const freePercent =
    volume.totalBytes > 0 ? Math.round((volume.availableBytes / volume.totalBytes) * 100) : 0;
  const usedPercent = 100 - freePercent;

  return (
    <button
      className={cn(
        "w-full rounded-md px-2 py-2 text-left transition-colors hover:bg-muted",
        isActive && "bg-accent text-accent-foreground",
      )}
      onClick={() => onNavigate(volume.mountPoint)}
      title={`${presentation.primary} · ${formatBytes(volume.availableBytes)} 可用，共 ${formatBytes(volume.totalBytes)}`}
      type="button"
    >
      <div className="flex items-center gap-2">
        {volume.isRemovable ? (
          <UsbIcon className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <HardDriveIcon className="size-4 shrink-0 text-muted-foreground" />
        )}
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm">{presentation.primary}</div>
          <div className="truncate text-xs text-muted-foreground">{presentation.secondary}</div>
        </div>
      </div>
      <div
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={usedPercent}
        className="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-muted"
        role="progressbar"
      >
        <div
          className={cn(
            "h-full rounded-full bg-primary transition-all",
            usedPercent > 90 && "bg-destructive",
          )}
          style={{ width: `${usedPercent}%` }}
        />
      </div>
      <div className="mt-1 flex justify-between gap-2 text-[11px] text-muted-foreground">
        <span className="shrink-0">剩余 {freePercent}%</span>
        <span className="truncate tabular-nums">
          {formatBytes(volume.availableBytes)} / 共 {formatBytes(volume.totalBytes)}
        </span>
      </div>
    </button>
  );
}

function SidebarItem({
  icon: Icon,
  isActive,
  label,
  onClick,
  title,
}: {
  icon: LucideIcon;
  isActive: boolean;
  label: string;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-muted",
        isActive && "bg-accent text-accent-foreground",
      )}
      onClick={onClick}
      title={title}
      type="button"
    >
      <Icon className={cn("size-4 shrink-0", !isActive && "text-muted-foreground")} />
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}

function SidebarSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mb-3">
      <h2 className="px-2 pb-1 text-xs font-medium text-muted-foreground">{title}</h2>
      {children}
    </div>
  );
}

function getDiskPresentation(volume: DiskVolume): { primary: string; secondary: string } {
  const driveLetter = /^([a-zA-Z]):[\\/]*$/.exec(volume.mountPoint)?.[1]?.toUpperCase();

  if (driveLetter) {
    const label = volume.name.trim();
    return {
      primary: label ? `${label} (${driveLetter}:)` : `本地磁盘 (${driveLetter}:)`,
      secondary: volume.fileSystem,
    };
  }

  return {
    primary: volume.name.trim() || volume.mountPoint,
    secondary: volume.mountPoint,
  };
}
