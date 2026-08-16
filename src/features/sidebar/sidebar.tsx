import {
  useEffect,
  useState,
  useSyncExternalStore,
  type ComponentProps,
  type ComponentType,
  type FormEvent,
  type ReactNode,
} from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ClipboardTextIcon,
  CloudIcon,
  CopyIcon,
  FolderOpenIcon,
  GlobeIcon,
  HardDriveIcon,
  HouseIcon,
  ClockCounterClockwiseIcon,
  PencilSimpleIcon,
  PlusIcon,
  ScissorsIcon,
  SquaresFourIcon,
  StarIcon,
  TabsIcon,
  TrashIcon,
  UsbIcon,
} from "@phosphor-icons/react";

import { commands, type StoredConnection } from "@/bindings";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Input } from "@/components/ui/input";
import {
  activeTabIdAtom,
  createTabWithSurfaceAtom,
  fileClipboardAtom,
  getTabNavigator,
  openInNewTabAtom,
} from "@/features/explorer/tabs";
import { spaceRenameRequestAtom } from "@/features/workspace/space-view";
import { getSpaceAccent } from "@/features/workspace/space-identity";
import { createSpace, ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import {
  activeSurfaceAtom,
  navigateToFolderAtom,
  openSurfaceAtom,
} from "@/features/workspace/workspace-atoms";
import { cn, formatBytes } from "@/lib/utils";

import { addFavoritePathsAtom, sidebarVisibleAtom } from "./sidebar-atoms";
import type { DiskVolume } from "./types";
import { ConnectDialog } from "./connect-dialog";
import { PenguinIcon } from "./penguin-icon";
import { ThemeMenu } from "./theme-menu";
import { useConnections } from "./use-connections";
import { useDiskVolumes } from "./use-disk-volumes";
import { useWslDistros } from "./use-wsl-distros";

/**
 * The persistent navigation rail. Follows the workspace information
 * architecture: Overview / Recents / Favorites, then Spaces, then Locations
 * (computer, network, cloud).
 */
export function Sidebar() {
  const visible = useAtomValue(sidebarVisibleAtom);
  if (!visible) return null;

  return <SidebarContent />;
}

function SidebarContent() {
  const surface = useAtomValue(activeSurfaceAtom);
  const openSurface = useSetAtom(openSurfaceAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const createTabWithSurface = useSetAtom(createTabWithSurfaceAtom);
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const setSpaceRenameRequest = useSetAtom(spaceRenameRequestAtom);
  const [creatingSpace, setCreatingSpace] = useState(false);
  const [spaceName, setSpaceName] = useState("");
  const [connectOpen, setConnectOpen] = useState(false);
  const activeTabId = useAtomValue(activeTabIdAtom);
  const navigator = getTabNavigator(activeTabId);
  const { directory } = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  // Location rows highlight only while the tab actually shows a folder.
  const currentPath = surface.kind === "folder" ? (directory?.path ?? null) : null;
  const volumes = useDiskVolumes(currentPath);
  const wslDistros = useWslDistros();
  const { connections, refresh: refreshConnections } = useConnections();

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  const submitCreateSpace = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = spaceName.trim();
    setCreatingSpace(false);
    setSpaceName("");
    if (!name) return;

    void createSpace(name).then((space) => {
      if (space) openSurface({ kind: "space", spaceId: space.id });
    });
  };

  return (
    <nav aria-label="侧边栏" className="flex w-56 shrink-0 flex-col border-r bg-background">
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        <NavItem
          icon={HouseIcon}
          isActive={surface.kind === "overview"}
          label="概览"
          onClick={() => openSurface({ kind: "overview" })}
        />
        <NavItem
          icon={ClockCounterClockwiseIcon}
          isActive={surface.kind === "recents"}
          label="最近使用"
          onClick={() => openSurface({ kind: "recents" })}
        />
        {/* File entries can be dragged onto Favorites; see drag-drop.ts. */}
        <div data-sidebar-favorites-drop-target="">
          <NavItem
            icon={StarIcon}
            isActive={surface.kind === "favorites"}
            label="收藏"
            onClick={() => openSurface({ kind: "favorites" })}
          />
        </div>

        <SectionLabel label="空间" onAdd={() => setCreatingSpace(true)} addTitle="新建空间" />
        {creatingSpace && (
          <form className="px-0.5 pb-1" onSubmit={submitCreateSpace}>
            <Input
              aria-label="空间名称"
              autoFocus
              className="h-7 text-[13px]"
              onBlur={() => {
                setCreatingSpace(false);
                setSpaceName("");
              }}
              onChange={(event) => setSpaceName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setCreatingSpace(false);
                  setSpaceName("");
                }
              }}
              placeholder="空间名称"
              value={spaceName}
            />
          </form>
        )}
        {spaces.map((space) => (
          // File entries can be dragged onto a Space; see drag-drop.ts.
          <div data-sidebar-space-drop-target={space.id} key={space.id}>
            <ContextMenu>
              <ContextMenuTrigger>
                <NavItem
                  icon={SquaresFourIcon}
                  iconClassName={getSpaceAccent(space.id).text}
                  isActive={surface.kind === "space" && surface.spaceId === space.id}
                  label={space.name}
                  onClick={() => openSurface({ kind: "space", spaceId: space.id })}
                />
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuGroup>
                  <ContextMenuItem
                    onClick={() => openSurface({ kind: "space", spaceId: space.id })}
                  >
                    <FolderOpenIcon />
                    打开
                  </ContextMenuItem>
                  <ContextMenuItem
                    onClick={() => createTabWithSurface({ kind: "space", spaceId: space.id })}
                  >
                    <TabsIcon />
                    在新标签页打开
                  </ContextMenuItem>
                </ContextMenuGroup>
                <ContextMenuSeparator />
                <ContextMenuGroup>
                  <ContextMenuItem
                    onClick={() => {
                      openSurface({ kind: "space", spaceId: space.id });
                      setSpaceRenameRequest(space.id);
                    }}
                  >
                    <PencilSimpleIcon />
                    重命名空间
                  </ContextMenuItem>
                </ContextMenuGroup>
              </ContextMenuContent>
            </ContextMenu>
          </div>
        ))}

        <SectionLabel label="位置" />
        <SubLabel label="计算机" />
        {volumes.map((volume) => (
          <DiskItem
            isActive={currentPath === volume.mountPoint}
            key={volume.mountPoint}
            onNavigate={navigateToFolder}
            volume={volume}
          />
        ))}
        {wslDistros.map((distro) => (
          <FolderContextMenu isListed={false} key={distro.path} path={distro.path}>
            <NavItem
              icon={PenguinIcon}
              isActive={currentPath === distro.path}
              label={distro.name}
              onClick={() => navigateToFolder(distro.path)}
              title={distro.path}
            />
          </FolderContextMenu>
        ))}

        <NetworkSection
          connections={connections}
          currentPath={currentPath}
          onAdd={() => setConnectOpen(true)}
          onNavigate={navigateToFolder}
          onRemoved={refreshConnections}
        />

        <SubLabel label="云存储" />
        <div className="flex items-center gap-2 rounded-2xs px-2.5 py-1.5 text-[13px] text-muted-foreground/70">
          <CloudIcon className="size-4 shrink-0" />
          <span className="text-xs">即将支持</span>
        </div>
      </div>

      <div className="flex shrink-0 items-center justify-end border-t px-2 py-1.5">
        <ThemeMenu />
      </div>

      <ConnectDialog
        onOpenChange={setConnectOpen}
        onSaved={(connection) => {
          refreshConnections();
          navigateToFolder(connection.id);
        }}
        open={connectOpen}
      />
    </nav>
  );
}

function SectionLabel({
  addTitle,
  label,
  onAdd,
}: {
  addTitle?: string;
  label: string;
  onAdd?: () => void;
}) {
  return (
    <div className="mt-4 flex items-center justify-between px-2.5 pb-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {onAdd && (
        <button
          aria-label={addTitle}
          className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
          onClick={onAdd}
          title={addTitle}
          type="button"
        >
          <PlusIcon className="size-4" />
        </button>
      )}
    </div>
  );
}

function SubLabel({ label }: { label: string }) {
  return <div className="mt-2 px-2.5 pb-0.5 text-[11px] text-muted-foreground/80">{label}</div>;
}

function NavItem({
  icon: Icon,
  iconClassName,
  isActive,
  label,
  onClick,
  title,
}: {
  icon: ComponentType<{ className?: string }>;
  iconClassName?: string;
  isActive: boolean;
  label: string;
  onClick: () => void;
  title?: string;
}) {
  return (
    <button
      aria-current={isActive ? "page" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-2xs px-2.5 py-1.5 text-left text-[13px] transition-colors hover:bg-muted/70",
        isActive && "bg-selection",
      )}
      onClick={onClick}
      title={title ?? label}
      type="button"
    >
      <Icon
        className={cn(
          "size-4 shrink-0",
          isActive ? "text-primary" : "text-muted-foreground",
          iconClassName,
        )}
      />
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}

function NetworkSection({
  connections,
  currentPath,
  onAdd,
  onNavigate,
  onRemoved,
}: {
  connections: StoredConnection[];
  currentPath: string | null;
  onAdd: () => void;
  onNavigate: (path: string) => void;
  onRemoved: () => void;
}) {
  return (
    <div className="mt-2">
      <div className="flex items-center justify-between px-2.5 pb-0.5">
        <span className="text-[11px] text-muted-foreground/80">网络</span>
        <button
          aria-label="连接网络存储"
          className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
          onClick={onAdd}
          title="连接网络存储"
          type="button"
        >
          <PlusIcon className="size-4" />
        </button>
      </div>

      {connections.map((connection) => (
        <ConnectionContextMenu
          key={connection.id}
          onRemoved={() => {
            void commands
              .deleteConnection(connection.id)
              .then(onRemoved)
              .catch((error: unknown) => console.warn("Unable to delete connection", error));
          }}
          path={connection.id}
        >
          <NavItem
            icon={GlobeIcon}
            isActive={
              currentPath === connection.id || currentPath?.startsWith(`${connection.id}/`) === true
            }
            label={connection.host}
            onClick={() => onNavigate(connection.id)}
            title={connection.id}
          />
        </ConnectionContextMenu>
      ))}

      {connections.length === 0 && (
        <p className="px-2.5 py-1.5 text-xs leading-relaxed text-muted-foreground">
          点击 + 连接 NAS 或 Windows 共享
        </p>
      )}
    </div>
  );
}

function ConnectionContextMenu({
  children,
  onRemoved,
  path,
}: {
  children: ReactNode;
  onRemoved: () => void;
  path: string;
}) {
  const setClipboard = useSetAtom(fileClipboardAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);

  return (
    <ContextMenu>
      <ContextMenuTrigger>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => openInNewTab(path)}>
            <FolderOpenIcon />
            在新标签页打开
          </ContextMenuItem>
          <ContextMenuItem onClick={() => void copyEntryPath(path)}>
            <ClipboardTextIcon />
            复制地址
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => setClipboard({ operation: "copy", sourcePaths: [path] })}>
            <CopyIcon />
            复制
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setClipboard({ operation: "cut", sourcePaths: [path] })}>
            <ScissorsIcon />
            剪切
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={onRemoved}>
            <TrashIcon />
            移除连接
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function FolderContextMenu({
  children,
  isListed,
  path,
  triggerProps,
}: {
  children: ReactNode;
  isListed: boolean;
  path: string;
  triggerProps?: ComponentProps<typeof ContextMenuTrigger>;
}) {
  const setClipboard = useSetAtom(fileClipboardAtom);
  const addFavoritePaths = useSetAtom(addFavoritePathsAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);

  return (
    <ContextMenu>
      <ContextMenuTrigger {...triggerProps}>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => openInNewTab(path)}>
            <FolderOpenIcon />
            在新标签页打开
          </ContextMenuItem>
          {!isListed && (
            <ContextMenuItem onClick={() => addFavoritePaths([path])}>
              <StarIcon />
              添加到收藏
            </ContextMenuItem>
          )}
          <ContextMenuItem onClick={() => void copyEntryPath(path)}>
            <ClipboardTextIcon />
            复制文件地址
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => setClipboard({ operation: "copy", sourcePaths: [path] })}>
            <CopyIcon />
            复制
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setClipboard({ operation: "cut", sourcePaths: [path] })}>
            <ScissorsIcon />
            剪切
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
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
        "w-full rounded-2xs px-2.5 py-1.5 text-left transition-colors hover:bg-muted/70",
        isActive && "bg-selection",
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
          <div className="truncate text-[13px]">{presentation.primary}</div>
          <div className="truncate text-xs text-muted-foreground">{presentation.secondary}</div>
        </div>
      </div>
      <div
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={usedPercent}
        className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-muted"
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

async function copyEntryPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
  }
}
