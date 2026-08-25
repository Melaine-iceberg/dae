import { useEffect, useMemo, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { openPath } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ClockCounterClockwiseIcon,
  ClipboardTextIcon,
  FolderIcon,
  FolderOpenIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";

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
  type PhosphorIcon,
} from "@/features/explorer/file-icons";
import { cn } from "@/lib/utils";

import {
  clearRecentItems,
  ensureRecentsLoadedAtom,
  recentsAtom,
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
              <span className="text-xs text-muted-foreground">{t("recents.clearNotice")}</span>
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
              <TrashIcon />
              {t("recents.clearAll")}
            </Button>
          ))
        }
        description={t("recents.description")}
        title={t("recents.title")}
      />

      {recents === null ? (
        <div className="flex flex-col gap-1">
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton className="h-9 rounded-lg" key={index} />
          ))}
        </div>
      ) : groups.length === 0 ? (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ClockCounterClockwiseIcon />
            </EmptyMedia>
            <EmptyTitle>{t("recents.emptyTitle")}</EmptyTitle>
            <EmptyDescription>{t("recents.emptyDescription")}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        groups.map((group) => (
          <section aria-label={group.label} key={group.label}>
            <h2 className="mb-1 text-label uppercase text-muted-foreground">{group.label}</h2>
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
  const Icon: PhosphorIcon = presentation.icon;
  const location = parentPathOf(item.path);

  return (
    <li>
      <ContextMenu>
        <ContextMenuTrigger>
          <button
            aria-label={`${presentation.label} ${item.name}`}
            className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            onClick={onOpen}
            title={item.path}
            type="button"
          >
            <Icon
              className={cn("size-4 shrink-0", getPresentationIconClassName(presentation))}
              weight={item.kind === "directory" ? "fill" : "regular"}
            />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px]">{item.name}</span>
              {location && (
                <span className="block truncate text-xs text-muted-foreground">{location}</span>
              )}
            </span>
            <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
              {formatTime(item.accessedAt)}
            </span>
          </button>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuGroup>
            <ContextMenuItem onClick={onOpen}>
              <FolderOpenIcon />
              {t("recents.open")}
            </ContextMenuItem>
            {location && (
              <ContextMenuItem onClick={onOpenContainingFolder}>
                <FolderIcon />
                {t("recents.openContainingFolder")}
              </ContextMenuItem>
            )}
            <ContextMenuItem onClick={() => void copyPath(item.path)}>
              <ClipboardTextIcon />
              {t("recents.copyPath")}
            </ContextMenuItem>
          </ContextMenuGroup>
          <ContextMenuSeparator />
          <ContextMenuGroup>
            <ContextMenuItem onClick={() => removeRecentItem(item.path)}>
              <XIcon />
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
