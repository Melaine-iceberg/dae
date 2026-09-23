import { useEffect, useMemo, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { openPath } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { History, ClipboardList, Folder, FolderOpen, Trash2, X } from "lucide-react";

import type { RecentItem } from "@/bindings";
import { i18n } from "@/i18n";

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
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorState } from "@/components/ui/error-state";
import { DIRECTORY_PRESENTATION, getFilePresentation } from "@/features/explorer/file-icons";
import { TypeIconTile } from "@/features/explorer/icon-tile";

import {
  clearRecentItems,
  ensureRecentsLoadedAtom,
  recentsAtom,
  recentsErrorAtom,
  recordRecentItem,
  removeRecentItem,
} from "./recents-atoms";
import { navigateToFolderAtom } from "./workspace-atoms";
import { WorkspacePage, WorkspacePageHeader, parentPathOf } from "./workspace-components";

type RecentGroup = { label: string; items: RecentItem[] };

/** Recent files and folders, grouped by day. Clearing only clears history. */
export function RecentsView() {
  const { t } = useTranslation("workspace");
  const recents = useAtomValue(recentsAtom);
  const recentsError = useAtomValue(recentsErrorAtom);
  const ensureRecentsLoaded = useSetAtom(ensureRecentsLoadedAtom);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);
  const [confirmingClear, setConfirmingClear] = useState(false);

  useEffect(() => {
    void ensureRecentsLoaded();
  }, [ensureRecentsLoaded]);

  const groups = useMemo(
    () =>
      groupByDay(recents ?? [], {
        today: t("recents.groups.today"),
        yesterday: t("recents.groups.yesterday"),
        thisWeek: t("recents.groups.thisWeek"),
        earlier: t("recents.groups.earlier"),
      }),
    [recents, t],
  );

  const openItem = (item: RecentItem) => {
    if (item.kind === "directory") {
      navigateToFolder(item.path);
      return;
    }

    recordRecentItem(item.path, item.kind, "opened");
    void openPath(item.path).catch((error: unknown) =>
      console.warn(`Unable to open ${item.path}`, error),
    );
  };

  const openContainingFolder = (item: RecentItem) => {
    const parent = parentPathOf(item.path);
    if (parent) navigateToFolder(parent);
  };

  return (
    <WorkspacePage aria-label={t("recents.title")}>
      <WorkspacePageHeader
        actions={
          (recents?.length ?? 0) > 0 &&
          (confirmingClear ? (
            <>
              <span className="text-caption text-muted-foreground">{t("recents.clearNotice")}</span>
              <Button
                onClick={() => {
                  clearRecentItems();
                  setConfirmingClear(false);
                }}
                size="sm"
                type="button"
                variant="destructive"
              >
                {t("recents.confirmClear")}
              </Button>
              <Button
                onClick={() => setConfirmingClear(false)}
                size="sm"
                type="button"
                variant="outline"
              >
                {t("recents.cancel")}
              </Button>
            </>
          ) : (
            <Button
              onClick={() => setConfirmingClear(true)}
              size="sm"
              type="button"
              variant="outline"
            >
              <Trash2 />
              {t("recents.clearAll")}
            </Button>
          ))
        }
        title={t("recents.title")}
      />

      {recentsError !== null ? (
        <ErrorState
          className="min-h-64"
          description={recentsError}
          onRetry={() => void ensureRecentsLoaded()}
          title={t("loadError.recentsTitle")}
        />
      ) : recents === null ? (
        <div className="flex flex-col gap-1">
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton className="h-8.5 rounded-sm" key={index} />
          ))}
        </div>
      ) : groups.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <History />
            </EmptyMedia>
            <EmptyTitle>{t("recents.emptyTitle")}</EmptyTitle>
          </EmptyHeader>
        </Empty>
      ) : (
        groups.map((group) => (
          <section aria-label={group.label} key={group.label}>
            <h2 className="mb-1 text-label text-muted-foreground">{group.label}</h2>
            <ul className="flex flex-col">
              {group.items.map((item) => (
                <RecentRow
                  item={item}
                  key={item.path}
                  onOpen={() => openItem(item)}
                  onOpenContainingFolder={() => openContainingFolder(item)}
                />
              ))}
            </ul>
          </section>
        ))
      )}
    </WorkspacePage>
  );
}

function RecentRow({
  item,
  onOpen,
  onOpenContainingFolder,
}: {
  item: RecentItem;
  onOpen: () => void;
  onOpenContainingFolder: () => void;
}) {
  const { t } = useTranslation("workspace");
  const presentation =
    item.kind === "directory" ? DIRECTORY_PRESENTATION : getFilePresentation(item.name);
  const location = parentPathOf(item.path);

  return (
    <li>
      <ContextMenu>
        <ContextMenuTrigger>
          <button
            aria-label={`${presentation.label} ${item.name}`}
            className="flex h-8.5 w-full items-center gap-2.5 rounded-sm px-2 text-left transition-colors duration-fast ease-standard hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            onClick={onOpen}
            title={item.path}
            type="button"
          >
            <TypeIconTile
              className="size-tile-list"
              iconSize={16}
              presentation={presentation}
            />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-body">{item.name}</span>
              {location && (
                <span className="block truncate text-caption text-muted-foreground">{location}</span>
              )}
            </span>
            <span className="shrink-0 text-caption text-muted-foreground tabular-nums">
              {formatTime(item.accessedAt)}
            </span>
          </button>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuGroup>
            <ContextMenuItem onClick={onOpen}>
              <FolderOpen />
              {t("recents.open")}
            </ContextMenuItem>
            {location && (
              <ContextMenuItem onClick={onOpenContainingFolder}>
                <Folder />
                {t("recents.openContainingFolder")}
              </ContextMenuItem>
            )}
            <ContextMenuItem onClick={() => void copyPath(item.path)}>
              <ClipboardList />
              {t("recents.copyPath")}
            </ContextMenuItem>
          </ContextMenuGroup>
          <ContextMenuSeparator />
          <ContextMenuGroup>
            <ContextMenuItem onClick={() => removeRecentItem(item.path)}>
              <X />
              {t("recents.remove")}
            </ContextMenuItem>
          </ContextMenuGroup>
        </ContextMenuContent>
      </ContextMenu>
    </li>
  );
}

function groupByDay(
  items: RecentItem[],
  labels: { today: string; yesterday: string; thisWeek: string; earlier: string },
): RecentGroup[] {
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfYesterday = startOfToday - 86_400_000;
  const startOfWeek = startOfToday - 6 * 86_400_000;

  const groups: RecentGroup[] = [];
  const push = (label: string, item: RecentItem) => {
    const last = groups.at(-1);
    if (last?.label === label) {
      last.items.push(item);
    } else {
      groups.push({ label, items: [item] });
    }
  };

  for (const item of items) {
    if (item.accessedAt >= startOfToday) {
      push(labels.today, item);
    } else if (item.accessedAt >= startOfYesterday) {
      push(labels.yesterday, item);
    } else if (item.accessedAt >= startOfWeek) {
      push(labels.thisWeek, item);
    } else {
      push(labels.earlier, item);
    }
  }

  return groups;
}

function formatTime(accessedAt: number): string {
  return new Date(accessedAt).toLocaleTimeString(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

async function copyPath(path: string): Promise<void> {
  try {
    await writeText(path);
  } catch (error) {
    console.warn(`Unable to copy path ${path}`, error);
  }
}
