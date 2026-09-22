/**
 * The explorer's toolbar: navigation, favorites, path bar, search, view
 * menus, Git branch, split toggle, preview toggle and the terminal toggle.
 *
 * The markup was lifted out of `explorer-view.tsx` verbatim; everything it
 * reads arrives as props, so the view computes state and this component only
 * lays it out. That split also keeps the toolbar's narrow-window overflow
 * behaviour (`TOOLBAR_OVERFLOW_CLASS`) documented in one place.
 */
import { useTranslation } from "react-i18next";
import { ArrowLeft, ArrowRight, ArrowUp, Columns3, Eye, PanelLeft, RotateCw, Star } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { cn } from "@/lib/utils";

import { ListingStats, TerminalToggle, ToolbarSeparator } from "./explorer-chrome";
import { type ContentSearchController } from "./content-search";
import {
  DirectorySearch,
  type DirectorySearchController,
  type ExplorerSearchMode,
} from "./directory-search";
import { ExplorerPathBar } from "./explorer-path-bar";
import { GitBranchControl } from "./git-branches";
import type { ExplorerGitStatus } from "./git-status";
import type { Breadcrumb, DirectoryView } from "./types";
import { ViewMenu } from "./view-menu";
import { FilterMenu } from "./filter-menu";

/**
 * Controls the toolbar gives up on a narrow window.
 *
 * The toolbar is a single non-wrapping row of `shrink-0` controls, and the
 * pane it lives in has `overflow-hidden` — so below a certain width the
 * trailing controls were not merely cramped, they were clipped and
 * unreachable. The path bar is `flex-1` and the search field shrinks, which
 * covers most of the range; what is left over is paid for by the two controls
 * that are reachable some other way: the favorite toggle (the folder's context
 * menu) and the split-view toggle (a per-tab layout choice, not a per-folder
 * one). Both come back as the window widens.
 */
const TOOLBAR_OVERFLOW_CLASS = "max-[880px]:hidden";

export interface ExplorerToolbarProps {
  sidebarVisible: boolean;
  onToggleSidebar: () => void;
  canGoBack: boolean;
  canGoForward: boolean;
  canGoUp: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  onGoUp: () => void;
  onRefresh: () => void;
  isLoading: boolean;
  directory: DirectoryView | null;
  isCurrentFavorited: boolean;
  onToggleFavorite: () => void;
  onNavigateBreadcrumb: (breadcrumb: Breadcrumb) => void;
  onNavigatePath: (path: string) => Promise<boolean>;
  stats: {
    isLoading: boolean;
    itemCount: number;
    searchError: string | null;
    searchQuery: string | null;
    selectedCount: number;
    truncated: boolean;
  };
  search: DirectorySearchController;
  contentSearch: ContentSearchController;
  searchMode: ExplorerSearchMode;
  onSearchModeChange: (mode: ExplorerSearchMode) => void;
  gitStatus: ExplorerGitStatus | null;
  splitEnabled: boolean;
  onToggleSplit?: () => void;
  isPreviewOpen: boolean;
  onTogglePreview: () => void;
}

export function ExplorerToolbar({
  sidebarVisible,
  onToggleSidebar,
  canGoBack,
  canGoForward,
  canGoUp,
  onGoBack,
  onGoForward,
  onGoUp,
  onRefresh,
  isLoading,
  directory,
  isCurrentFavorited,
  onToggleFavorite,
  onNavigateBreadcrumb,
  onNavigatePath,
  stats,
  search,
  contentSearch,
  searchMode,
  onSearchModeChange,
  gitStatus,
  splitEnabled,
  onToggleSplit,
  isPreviewOpen,
  onTogglePreview,
}: ExplorerToolbarProps) {
  const { t } = useTranslation("explorer");
  // Live binding for the preview toggle's tooltip. It used to be baked into
  // the translation string ("收起预览面板 (Space)"), which meant a rebind left
  // the tooltip teaching a key that no longer did anything.
  const previewBinding = formatBinding(useBinding("explorer.preview"));

  return (
    <header
      className="flex h-toolbar shrink-0 items-center gap-0.5 border-b border-border bg-card px-1.5"
      data-tauri-drag-region="deep"
    >
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          aria-label={
            sidebarVisible ? t("explorer:toolbar.hideSidebar") : t("explorer:toolbar.showSidebar")
          }
          onClick={onToggleSidebar}
          size="icon"
          title={
            sidebarVisible ? t("explorer:toolbar.hideSidebar") : t("explorer:toolbar.showSidebar")
          }
          type="button"
          variant="ghost"
        >
          <PanelLeft />
        </Button>
        <ToolbarSeparator />
        <Button
          aria-label={t("explorer:toolbar.back")}
          disabled={!canGoBack}
          onClick={onGoBack}
          size="icon"
          title={t("explorer:toolbar.back")}
          type="button"
          variant="ghost"
        >
          <ArrowLeft />
        </Button>
        <Button
          aria-label={t("explorer:toolbar.forward")}
          disabled={!canGoForward}
          onClick={onGoForward}
          size="icon"
          title={t("explorer:toolbar.forward")}
          type="button"
          variant="ghost"
        >
          <ArrowRight />
        </Button>
        <Button
          aria-label={t("explorer:toolbar.up")}
          disabled={!canGoUp}
          onClick={onGoUp}
          size="icon"
          title={t("explorer:toolbar.up")}
          type="button"
          variant="ghost"
        >
          <ArrowUp />
        </Button>
        <Button
          aria-label={t("explorer:toolbar.refresh")}
          disabled={isLoading || !directory}
          onClick={onRefresh}
          size="icon"
          title={t("explorer:toolbar.refresh")}
          type="button"
          variant="ghost"
        >
          <RotateCw className={cn(isLoading && "animate-spin")} />
        </Button>
        <ToolbarSeparator className={TOOLBAR_OVERFLOW_CLASS} />
        <Button
          aria-label={
            isCurrentFavorited
              ? t("explorer:toolbar.removeFavorite")
              : t("explorer:toolbar.addFavorite")
          }
          className={TOOLBAR_OVERFLOW_CLASS}
          disabled={!directory}
          onClick={onToggleFavorite}
          size="icon"
          title={
            isCurrentFavorited
              ? t("explorer:toolbar.removeFavorite")
              : t("explorer:toolbar.addFavorite")
          }
          type="button"
          variant="ghost"
        >
          <Star className={cn(isCurrentFavorited && "fill-warning/70 text-warning")} />
        </Button>
      </div>

      <div className="min-w-0 flex-1 px-1">
        {directory ? (
          <ExplorerPathBar
            directory={directory}
            onNavigate={onNavigateBreadcrumb}
            onNavigatePath={onNavigatePath}
            trailing={<ListingStats {...stats} />}
          />
        ) : (
          <Skeleton className="h-6 w-56 max-w-full" />
        )}
      </div>

      <DirectorySearch
        contentSearch={contentSearch}
        directoryName={directory?.breadcrumbs.at(-1)?.name ?? null}
        disabled={isLoading}
        mode={searchMode}
        onModeChange={onSearchModeChange}
        search={search}
      />
      <ViewMenu disabled={!directory} />
      <FilterMenu disabled={!directory} />
      {gitStatus && (
        <>
          <ToolbarSeparator />
          <GitBranchControl branch={gitStatus.branch} root={gitStatus.root} />
        </>
      )}
      {onToggleSplit && (
        <Button
          aria-label={
            splitEnabled ? t("explorer:toolbar.closeSplitView") : t("explorer:toolbar.splitView")
          }
          aria-pressed={splitEnabled}
          className={TOOLBAR_OVERFLOW_CLASS}
          onClick={onToggleSplit}
          size="icon"
          title={
            splitEnabled ? t("explorer:toolbar.closeSplitView") : t("explorer:toolbar.splitView")
          }
          type="button"
          variant="ghost"
        >
          <Columns3 />
        </Button>
      )}
      <ToolbarSeparator />
      <Button
        aria-label={
          isPreviewOpen
            ? t("explorer:toolbar.collapsePreview")
            : t("explorer:toolbar.expandPreview")
        }
        aria-pressed={isPreviewOpen}
        onClick={onTogglePreview}
        size="icon"
        title={
          isPreviewOpen
            ? t("explorer:toolbar.collapsePreviewShortcut", { shortcut: previewBinding })
            : t("explorer:toolbar.expandPreviewShortcut", { shortcut: previewBinding })
        }
        type="button"
        variant="ghost"
      >
        <Eye />
      </Button>
      <ToolbarSeparator />
      <TerminalToggle />
    </header>
  );
}
