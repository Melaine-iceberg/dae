import {
  useEffect,
  useState,
  useSyncExternalStore,
  type ComponentType,
  type FormEvent,
  type ReactNode,
} from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  CaretDownIcon,
  ClipboardTextIcon,
  CopyIcon,
  FolderOpenIcon,
  GlobeIcon,
  HardDriveIcon,
  HouseIcon,
  LinuxLogoIcon,
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

import { commands, type Breadcrumb, type StoredCloudAccount } from "@/bindings";

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
  activePaneNavigatorAtom,
  createTabWithSurfaceAtom,
  fileClipboardAtom,
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
import { i18n } from "@/i18n";
import { cn, formatBytes } from "@/lib/utils";

import {
  cloudAccountsAtom,
  collapsedLocationSectionsAtom,
  connectionsAtom,
  ensureCloudAccountsLoadedAtom,
  ensureConnectionsLoadedAtom,
  expandLocationSectionAtom,
  reloadCloudAccountsAtom,
  reloadConnectionsAtom,
  sidebarVisibleAtom,
  toggleLocationSectionAtom,
  type LocationSectionId,
} from "./sidebar-atoms";
import type { DiskVolume } from "./types";
import { CloudAccountDialog } from "./cloud-account-dialog";
import { CLOUD_PROVIDER_ICONS } from "./cloud-icons";
import { ConnectDialog } from "./connect-dialog";
import {
  FolderTree,
  ensureTreeNodeExpandedAtom,
  isPathWithin,
  toggleTreeNodeAtom,
  treeExpandedPathsAtom,
} from "./directory-tree";
import { FolderContextMenu, copyEntryPath } from "./folder-context-menu";
import { LanguageMenu } from "@/i18n/language-menu";
import { CloudSectionIcon, DisksSectionIcon, NetworkSectionIcon } from "./location-icons";
import { ThemeMenu } from "./theme-menu";
import { useDiskVolumes } from "./use-disk-volumes";
import { useWslDistros } from "./use-wsl-distros";
import { WslIcon } from "./wsl-icon";

/** WSL only exists on Windows; elsewhere the section is hidden entirely. */
const IS_WINDOWS = navigator.userAgent.includes("Windows");

/** Matches the grid-rows collapse transition (duration-normal). */
const COLLAPSE_ANIMATION_MS = 160;

/** Stable empty breadcrumb list for surfaces that are not showing a folder. */
const EMPTY_BREADCRUMBS: readonly Breadcrumb[] = [];

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
  const { t } = useTranslation("sidebar");
  const surface = useAtomValue(activeSurfaceAtom);
  const openSurface = useSetAtom(openSurfaceAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const createTabWithSurface = useSetAtom(createTabWithSurfaceAtom);
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);
  const setSpaceRenameRequest = useSetAtom(spaceRenameRequestAtom);
  const reloadConnections = useSetAtom(reloadConnectionsAtom);
  const reloadCloudAccounts = useSetAtom(reloadCloudAccountsAtom);
  const expandLocationSection = useSetAtom(expandLocationSectionAtom);
  const [creatingSpace, setCreatingSpace] = useState(false);
  const [spaceName, setSpaceName] = useState("");
  const [connectOpen, setConnectOpen] = useState(false);
  const [cloudOpen, setCloudOpen] = useState(false);
  // Location rows highlight the focused pane's folder in the active tab.
  const navigator = useAtomValue(activePaneNavigatorAtom);
  const { directory } = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  // Location rows highlight only while the tab actually shows a folder.
  const currentPath = surface.kind === "folder" ? (directory?.path ?? null) : null;

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
    <nav
      aria-label={t("nav.label")}
      className="flex w-56 shrink-0 flex-col overflow-hidden rounded-xl border bg-sidebar shadow-ambient-xs"
    >
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        <NavItem
          icon={HouseIcon}
          isActive={surface.kind === "overview"}
          label={t("nav.overview")}
          onClick={() => openSurface({ kind: "overview" })}
        />
        <NavItem
          icon={ClockCounterClockwiseIcon}
          isActive={surface.kind === "recents"}
          label={t("nav.recents")}
          onClick={() => openSurface({ kind: "recents" })}
        />
        {/* File entries can be dragged onto Favorites; see drag-drop.ts. */}
        <div data-sidebar-favorites-drop-target="">
          <NavItem
            icon={StarIcon}
            isActive={surface.kind === "favorites"}
            label={t("nav.favorites")}
            onClick={() => openSurface({ kind: "favorites" })}
          />
        </div>

        <SectionLabel
          label={t("sections.spaces")}
          onAdd={() => setCreatingSpace(true)}
          addTitle={t("spaces.create")}
        />
        {creatingSpace && (
          <form className="px-0.5 pb-1" onSubmit={submitCreateSpace}>
            <Input
              aria-label={t("spaces.name")}
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
              placeholder={t("spaces.name")}
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
                    {t("contextMenu.open")}
                  </ContextMenuItem>
                  <ContextMenuItem
                    onClick={() => createTabWithSurface({ kind: "space", spaceId: space.id })}
                  >
                    <TabsIcon />
                    {t("contextMenu.openInNewTab")}
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
                    {t("contextMenu.renameSpace")}
                  </ContextMenuItem>
                </ContextMenuGroup>
              </ContextMenuContent>
            </ContextMenu>
          </div>
        ))}

        <SectionLabel label={t("sections.locations")} />

        {/* Location groups are collapsed by default and mount their content
            (and its backend queries) only on first expand, keeping cold start
            free of disk enumeration, the `wsl.exe` probe, and store reads. */}
        <CollapsibleSection icon={DisksSectionIcon} id="disks" label={t("sections.disks")}>
          <DisksContent
            currentBreadcrumbs={directory?.breadcrumbs ?? EMPTY_BREADCRUMBS}
            currentPath={currentPath}
            onNavigate={navigateToFolder}
          />
        </CollapsibleSection>

        {IS_WINDOWS && (
          <CollapsibleSection icon={WslIcon} id="wsl" label={t("sections.wsl")}>
            <WslContent currentPath={currentPath} onNavigate={navigateToFolder} />
          </CollapsibleSection>
        )}

        <CollapsibleSection
          action={{
            label: t("network.connectStorage"),
            onClick: () => setConnectOpen(true),
          }}
          icon={NetworkSectionIcon}
          id="network"
          label={t("sections.network")}
        >
          <NetworkContent currentPath={currentPath} onNavigate={navigateToFolder} />
        </CollapsibleSection>

        <CollapsibleSection
          action={{
            label: t("cloud.addAccount"),
            onClick: () => setCloudOpen(true),
          }}
          icon={CloudSectionIcon}
          id="cloud"
          label={t("sections.cloudStorage")}
        >
          <CloudContent currentPath={currentPath} onNavigate={navigateToFolder} />
        </CollapsibleSection>
      </div>

      <div className="flex shrink-0 items-center justify-end gap-1 border-t border-sidebar-border px-2 py-1.5">
        <ThemeMenu />
        <LanguageMenu />
      </div>

      <ConnectDialog
        onOpenChange={setConnectOpen}
        onSaved={(connection) => {
          void reloadConnections();
          expandLocationSection("network");
          navigateToFolder(connection.id);
        }}
        open={connectOpen}
      />

      <CloudAccountDialog
        onAuthorized={(account) => {
          void reloadCloudAccounts();
          expandLocationSection("cloud");
          navigateToFolder(account.id);
        }}
        onOpenChange={setCloudOpen}
        open={cloudOpen}
      />
    </nav>
  );
}

/**
 * A collapsible location group: a compact header row with a leading icon and
 * a rotating chevron; the body expands with a grid-rows transition. Children
 * stay unmounted while collapsed (and are only mounted once expanded), so
 * collapsed groups cost zero IPC and zero render work.
 */
function CollapsibleSection({
  action,
  children,
  icon: Icon,
  id,
  label,
}: {
  action?: { label: string; onClick: () => void };
  children: ReactNode;
  icon: ComponentType<{ className?: string }>;
  id: LocationSectionId;
  label: string;
}) {
  const collapsed = useAtomValue(collapsedLocationSectionsAtom);
  const toggle = useSetAtom(toggleLocationSectionAtom);
  const open = !collapsed[id];

  // Keep the body mounted for the duration of the collapse animation, then
  // unmount so its hooks stop polling/querying the backend.
  const [contentMounted, setContentMounted] = useState(open);
  useEffect(() => {
    if (open) {
      setContentMounted(true);
      return;
    }
    const timer = window.setTimeout(() => setContentMounted(false), COLLAPSE_ANIMATION_MS);
    return () => window.clearTimeout(timer);
  }, [open]);

  return (
    <div className="mt-1">
      <div className="flex items-center gap-0.5">
        <button
          aria-expanded={open}
          className="group flex min-w-0 flex-1 items-center gap-2.5 rounded-sm px-2 py-1 text-left text-[13px] font-medium transition-[background-color,color] duration-fast ease-spring-fast hover:bg-accent/70"
          onClick={() => toggle(id)}
          type="button"
        >
          <Icon className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground" />
          <span className="min-w-0 flex-1 truncate">{label}</span>
          <CaretDownIcon
            aria-hidden="true"
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground/80 transition-transform duration-fast ease-spring-fast",
              !open && "-rotate-90",
            )}
          />
        </button>
        {action && (
          <button
            aria-label={action.label}
            className="shrink-0 rounded-xs p-0.5 text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
            onClick={action.onClick}
            title={action.label}
            type="button"
          >
            <PlusIcon className="size-4" />
          </button>
        )}
      </div>
      <div
        className={cn(
          "grid transition-[grid-template-rows] duration-normal ease-spring-fast",
          open ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
        )}
      >
        <div className="min-h-0 overflow-hidden">{contentMounted && children}</div>
      </div>
    </div>
  );
}

/** Placeholder row shown while a lazily mounted section runs its first query. */
function SectionSkeleton() {
  return (
    <div aria-hidden="true" className="px-2 py-1">
      <div className="h-7 animate-pulse rounded-sm bg-muted/70" />
    </div>
  );
}

function DisksContent({
  currentBreadcrumbs,
  currentPath,
  onNavigate,
}: {
  currentBreadcrumbs: readonly Breadcrumb[];
  currentPath: string | null;
  onNavigate: (path: string) => void;
}) {
  const volumes = useDiskVolumes(currentPath);
  const ensureTreeExpanded = useSetAtom(ensureTreeNodeExpandedAtom);

  // Follow the active pane: reveal and expand the tree path leading to the
  // folder currently shown, so the tree mirrors breadcrumb navigation.
  useEffect(() => {
    if (!volumes || !currentPath) return;
    const volume = volumes.find((candidate) => isPathWithin(currentPath, candidate.mountPoint));
    if (!volume) return;

    ensureTreeExpanded(volume.mountPoint);
    for (const crumb of currentBreadcrumbs) {
      if (crumb.path !== currentPath && isPathWithin(crumb.path, volume.mountPoint)) {
        ensureTreeExpanded(crumb.path);
      }
    }
  }, [currentBreadcrumbs, currentPath, ensureTreeExpanded, volumes]);

  if (volumes === null) return <SectionSkeleton />;

  return volumes.map((volume) => (
    <DiskItem
      currentPath={currentPath}
      isActive={currentPath === volume.mountPoint}
      key={volume.mountPoint}
      onNavigate={onNavigate}
      volume={volume}
    />
  ));
}

function WslContent({
  currentPath,
  onNavigate,
}: {
  currentPath: string | null;
  onNavigate: (path: string) => void;
}) {
  const { t } = useTranslation("sidebar");
  const distros = useWslDistros();
  if (distros === null) return <SectionSkeleton />;

  if (distros.length === 0) {
    return <p className="px-3.5 py-1.5 text-xs text-muted-foreground">{t("wsl.empty")}</p>;
  }

  return distros.map((distro) => (
    <FolderContextMenu isListed={false} key={distro.path} path={distro.path}>
      <NavItem
        icon={LinuxLogoIcon}
        isActive={currentPath === distro.path}
        label={distro.name}
        onClick={() => onNavigate(distro.path)}
        title={distro.path}
      />
    </FolderContextMenu>
  ));
}

function NetworkContent({
  currentPath,
  onNavigate,
}: {
  currentPath: string | null;
  onNavigate: (path: string) => void;
}) {
  const { t } = useTranslation("sidebar");
  const connections = useAtomValue(connectionsAtom);
  const ensureLoaded = useSetAtom(ensureConnectionsLoadedAtom);

  useEffect(() => {
    void ensureLoaded();
  }, [ensureLoaded]);

  if (connections === null) return <SectionSkeleton />;

  return (
    <>
      {connections.map((connection) => (
        <ConnectionContextMenu key={connection.id} connection={connection}>
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
        <p className="px-3.5 py-1.5 text-xs leading-relaxed text-muted-foreground">
          {t("network.emptyHint")}
        </p>
      )}
    </>
  );
}

function CloudContent({
  currentPath,
  onNavigate,
}: {
  currentPath: string | null;
  onNavigate: (path: string) => void;
}) {
  const { t } = useTranslation("sidebar");
  const accounts = useAtomValue(cloudAccountsAtom);
  const ensureLoaded = useSetAtom(ensureCloudAccountsLoadedAtom);

  useEffect(() => {
    void ensureLoaded();
  }, [ensureLoaded]);

  if (accounts === null) return <SectionSkeleton />;

  return (
    <>
      {accounts.map((account) => {
        const Icon = CLOUD_PROVIDER_ICONS[account.provider];
        return (
          <CloudAccountContextMenu account={account} key={account.id}>
            <NavItem
              icon={Icon}
              isActive={
                currentPath === account.id || currentPath?.startsWith(`${account.id}/`) === true
              }
              label={account.email}
              onClick={() => onNavigate(account.id)}
              title={account.displayName ? `${account.displayName} (${account.email})` : account.id}
            />
          </CloudAccountContextMenu>
        );
      })}

      {accounts.length === 0 && (
        <p className="px-3.5 py-1.5 text-xs leading-relaxed text-muted-foreground">
          {t("cloud.emptyHint")}
        </p>
      )}
    </>
  );
}

function CloudAccountContextMenu({
  account,
  children,
}: {
  account: StoredCloudAccount;
  children: ReactNode;
}) {
  const { t } = useTranslation("sidebar");
  const openInNewTab = useSetAtom(openInNewTabAtom);
  const reloadCloudAccounts = useSetAtom(reloadCloudAccountsAtom);
  const path = account.id;

  const remove = () => {
    void commands
      .deleteCloudAccount(path)
      .then(() => reloadCloudAccounts())
      .catch((error: unknown) => console.warn("Unable to delete cloud account", error));
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => openInNewTab(path)}>
            <FolderOpenIcon />
            {t("contextMenu.openInNewTab")}
          </ContextMenuItem>
          <ContextMenuItem onClick={() => void copyEntryPath(path)}>
            <ClipboardTextIcon />
            {t("contextMenu.copyPath")}
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={remove}>
            <TrashIcon />
            {t("cloud.removeAccount")}
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
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
    <div className="mt-3 flex items-center justify-between px-2 pb-1">
      <span className="text-label uppercase text-muted-foreground">{label}</span>
      {onAdd && (
        <button
          aria-label={addTitle}
          className="rounded-xs p-0.5 text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
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
        "relative flex w-full items-center gap-2.5 rounded-sm px-2 py-1 text-left text-[13px] transition-[background-color,color] duration-fast ease-spring-fast hover:bg-accent/70",
        isActive && "bg-selection font-medium text-accent-foreground",
      )}
      onClick={onClick}
      title={title ?? label}
      type="button"
    >
      {/* Active indicator: a compact primary tick on the leading edge. */}
      <span
        aria-hidden="true"
        className={cn(
          "absolute left-0.5 h-4 w-[3px] rounded-xs bg-primary transition-[transform,opacity] duration-fast ease-spring-fast",
          isActive ? "scale-y-100 opacity-100" : "scale-y-50 opacity-0",
        )}
      />
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

function ConnectionContextMenu({
  children,
  connection,
}: {
  children: ReactNode;
  connection: { id: string };
}) {
  const { t } = useTranslation("sidebar");
  const setClipboard = useSetAtom(fileClipboardAtom);
  const openInNewTab = useSetAtom(openInNewTabAtom);
  const reloadConnections = useSetAtom(reloadConnectionsAtom);
  const path = connection.id;

  const remove = () => {
    void commands
      .deleteConnection(path)
      .then(() => reloadConnections())
      .catch((error: unknown) => console.warn("Unable to delete connection", error));
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => openInNewTab(path)}>
            <FolderOpenIcon />
            {t("contextMenu.openInNewTab")}
          </ContextMenuItem>
          <ContextMenuItem onClick={() => void copyEntryPath(path)}>
            <ClipboardTextIcon />
            {t("contextMenu.copyPath")}
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={() => setClipboard({ operation: "copy", sourcePaths: [path] })}>
            <CopyIcon />
            {t("contextMenu.copy")}
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setClipboard({ operation: "cut", sourcePaths: [path] })}>
            <ScissorsIcon />
            {t("contextMenu.cut")}
          </ContextMenuItem>
        </ContextMenuGroup>
        <ContextMenuSeparator />
        <ContextMenuGroup>
          <ContextMenuItem onClick={remove}>
            <TrashIcon />
            {t("contextMenu.removeConnection")}
          </ContextMenuItem>
        </ContextMenuGroup>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function DiskItem({
  currentPath,
  isActive,
  onNavigate,
  volume,
}: {
  currentPath: string | null;
  isActive: boolean;
  onNavigate: (path: string) => void;
  volume: DiskVolume;
}) {
  const { t } = useTranslation("sidebar");
  const treeOpen = useAtomValue(treeExpandedPathsAtom).has(volume.mountPoint);
  const toggleTreeNode = useSetAtom(toggleTreeNodeAtom);
  const presentation = getDiskPresentation(volume);
  const freePercent =
    volume.totalBytes > 0 ? Math.round((volume.availableBytes / volume.totalBytes) * 100) : 0;
  const usedPercent = 100 - freePercent;
  const title = t("disk.title", {
    name: presentation.primary,
    free: formatBytes(volume.availableBytes),
    total: formatBytes(volume.totalBytes),
  });

  return (
    <div
      className={cn(
        "w-full rounded-md px-2.5 py-2 transition-[background-color] duration-fast ease-spring-fast hover:bg-accent/60",
        isActive && "bg-selection",
      )}
    >
      <div className="flex items-center gap-1">
        <button
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          onClick={() => onNavigate(volume.mountPoint)}
          title={title}
          type="button"
        >
          {volume.isRemovable ? (
            <UsbIcon className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <HardDriveIcon className="size-4 shrink-0 text-muted-foreground" />
          )}
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13px]">{presentation.primary}</div>
            <div className="truncate text-xs text-muted-foreground">{presentation.secondary}</div>
          </div>
        </button>
        <button
          aria-expanded={treeOpen}
          aria-label={t(treeOpen ? "tree.collapse" : "tree.expand")}
          className="shrink-0 rounded-xs p-0.5 text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
          onClick={() => toggleTreeNode(volume.mountPoint)}
          type="button"
        >
          <CaretDownIcon
            aria-hidden="true"
            className={cn(
              "size-3.5 transition-transform duration-fast ease-spring-fast",
              !treeOpen && "-rotate-90",
            )}
          />
        </button>
      </div>
      {/* Secondary click target so the capacity block still navigates; the
          name row above carries keyboard focus for the same action. */}
      <button
        className="mt-1.5 block w-full text-left"
        onClick={() => onNavigate(volume.mountPoint)}
        tabIndex={-1}
        type="button"
      >
        <div
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={usedPercent}
          className="h-1 w-full overflow-hidden rounded-xs bg-muted"
          role="progressbar"
        >
          <div
            className={cn(
              "h-full rounded-xs bg-primary transition-all duration-normal",
              usedPercent > 90 && "bg-destructive",
            )}
            style={{ width: `${usedPercent}%` }}
          />
        </div>
        <div className="mt-1 flex justify-between gap-2 text-[11px] text-muted-foreground">
          <span className="shrink-0">{t("disk.freePercent", { percent: freePercent })}</span>
          <span className="truncate font-mono tabular-nums">
            {t("disk.capacity", {
              free: formatBytes(volume.availableBytes),
              total: formatBytes(volume.totalBytes),
            })}
          </span>
        </div>
      </button>
      {treeOpen && (
        <div className="mt-1.5">
          <FolderTree
            currentPath={currentPath}
            onNavigate={onNavigate}
            rootPath={volume.mountPoint}
          />
        </div>
      )}
    </div>
  );
}

function getDiskPresentation(volume: DiskVolume): { primary: string; secondary: string } {
  const driveLetter = /^([a-zA-Z]):[\\/]*$/.exec(volume.mountPoint)?.[1]?.toUpperCase();

  if (driveLetter) {
    const label = volume.name.trim();
    return {
      primary: label
        ? `${label} (${driveLetter}:)`
        : i18n.t("sidebar:disk.localDisk", { letter: driveLetter }),
      secondary: volume.fileSystem,
    };
  }

  return {
    primary: volume.name.trim() || volume.mountPoint,
    secondary: volume.mountPoint,
  };
}
