/**
 * The explorer's toolbar: navigation, favorites, path bar, search, view
 * menus, Git branch, split toggle, preview toggle and the terminal toggle.
 *
 * ONE ROW, PATH FIRST. The toolbar used to be two stacked rows — a path strip
 * over a row of controls — because the breadcrumbs were the first thing to be
 * squeezed when everything shared a line. That solved the squeeze by spending
 * a whole band of window height on chrome, which is the more expensive half of
 * the trade: a file manager's most valuable pixels are the ones in the listing.
 *
 * So the row is laid out by what a control actually does, left to right:
 *
 *   chrome   the sidebar toggle, alone at the leading edge
 *   travel   back / forward / up, in one recessed cluster
 *   place    the path field — the row's spine, `flex-1`
 *   find     the name/content search capsule
 *   act      reload, favorite
 *   view     view menu, filter menu, the folder's git branch
 *   panels   split, preview, terminal — pinned to the trailing edge
 *
 * The path field is a filled capsule rather than bare crumbs: it is the one
 * control in the row that holds a *value*, and a field that looks like a field
 * is what makes "click to edit the path" discoverable without a tooltip.
 *
 * Controls that give up on a narrow window are listed in
 * `TOOLBAR_OVERFLOW_CLASS` and friends: the pane is `overflow-hidden`, so a
 * control that does not fit is not merely cramped, it is unreachable.
 */
import { useTranslation } from "react-i18next";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  Columns3Icon,
  EyeIcon,
  PanelLeftIcon,
  RefreshIcon,
  StarIcon,
} from "@solar-icons/react/linear";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { cn } from "@/lib/utils";

import { ListingStats, TerminalToggle, TOGGLE_PRESSED_CLASS } from "./explorer-chrome";
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
 * Controls the toolbar gives up on a narrow window, in the order it gives them
 * up. Each is reachable some other way:
 *
 *   favorite  the folder's own context menu
 *   split     a per-tab layout choice, not a per-folder one
 *   git       the branch is context, not a control
 */
const TOOLBAR_OVERFLOW_CLASS = "max-[900px]:hidden";
const TOOLBAR_OVERFLOW_FAVORITE_CLASS = "max-[1080px]:hidden";
const TOOLBAR_OVERFLOW_GIT_CLASS = "max-[1180px]:hidden";

/** A cluster of controls that reads as one object: a recessed plate holding
 *  evenly weighted keys. Groups are told apart by that plate and by
 *  whitespace — never by a hairline between neighbours, which turns a row of
 *  icons into a picket fence.
 *
 *  The plate is Raycast's `.seg`: the void mixed one step toward the paper, plus
 *  a single inset hairline. It is *darker* than the toolbar it sits on, so a
 *  cluster reads as a slot milled into the frame rather than as a card lying on
 *  it — which is what a segmented control is. */
const CLUSTER_CLASS =
  "flex shrink-0 items-center gap-0.5 rounded-md bg-background/60 p-0.5 shadow-[inset_0_0_0_1px_var(--border)]";

/** Travel is the one group with no plate of its own: three bare glyphs at the
 *  leading edge, the way the reference draws back / forward / up. The row
 *  already spends its "one object" budget on the path field; a second plate
 *  beside it made the leading edge twice as heavy as the trailing one. */
const NAV_CLUSTER_CLASS = "flex shrink-0 items-center gap-0.5";

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
  /** Whether this pane is the one that owns keyboard shortcuts — passed down
   *  to the search field so its Mod+F listener answers for this pane only. */
  isActivePane: boolean;
  /** Incremented by the pane's Ctrl+L / Alt+D; forwarded to the path bar so
   *  its inline editor opens. */
  pathEditSignal: number;
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
  isActivePane,
  pathEditSignal,
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
      className="flex h-toolbar shrink-0 items-center gap-2 px-3"
      data-tauri-drag-region="deep"
    >
      {/* App chrome: the sidebar toggle, alone at the leading edge. */}
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
        <PanelLeftIcon />
      </Button>

      {/* Travel: where you have been and where "up" is. Three bare glyphs —
          the plate this group used to carry is gone, see NAV_CLUSTER_CLASS. */}
      <div className={NAV_CLUSTER_CLASS}>
        <Button
          aria-label={t("explorer:toolbar.back")}
          disabled={!canGoBack}
          onClick={onGoBack}
          size="icon"
          title={t("explorer:toolbar.back")}
          type="button"
          variant="ghost"
        >
          <ArrowLeftIcon />
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
          <ArrowRightIcon />
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
          <ArrowUpIcon />
        </Button>
      </div>

      {/* Place: the path field. It is the row's spine — the one control that
          takes every pixel the others do not — and it holds the listing stats
          on its trailing edge, because those facts describe the folder the
          crumbs name. */}
      <div
        className={cn(
          "flex h-8 min-w-40 flex-1 items-center rounded-md bg-background/60 px-1",
          // The field is a slot, not a card: one inset hairline at rest, a
          // brighter one under the pointer, and the focus ring only while it is
          // actually being typed into. `background-color` never changes — the
          // field holds a *value*, and a value does not change plane on hover.
          "shadow-[inset_0_0_0_1px_var(--border)] transition-[box-shadow] duration-fast ease-standard",
          "hover:shadow-[inset_0_0_0_1px_rgba(255,255,255,0.1)] focus-within:shadow-[inset_0_0_0_1px_rgba(255,255,255,0.16),0_0_0_3px_color-mix(in_oklab,var(--ring)_35%,transparent)]",
        )}
      >
        {directory ? (
          <ExplorerPathBar
            directory={directory}
            editSignal={pathEditSignal}
            onNavigate={onNavigateBreadcrumb}
            onNavigatePath={onNavigatePath}
            trailing={<ListingStats {...stats} />}
          />
        ) : (
          <Skeleton className="mx-3 h-4 w-40" />
        )}
      </div>

      {/* Find: the search capsule, same height and corner as the path field. */}
      <DirectorySearch
        contentSearch={contentSearch}
        directoryName={directory?.breadcrumbs.at(-1)?.name ?? null}
        disabled={isLoading || !isActivePane}
        mode={searchMode}
        onModeChange={onSearchModeChange}
        search={search}
      />

      {/* Act: the two things you do to the folder you are in — reload it,
          keep it. */}
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          aria-label={t("explorer:toolbar.refresh")}
          disabled={isLoading || !directory}
          onClick={onRefresh}
          size="icon"
          title={t("explorer:toolbar.refresh")}
          type="button"
          variant="ghost"
        >
          <RefreshIcon className={cn(isLoading && "animate-spin")} />
        </Button>
        <Button
          aria-label={
            isCurrentFavorited
              ? t("explorer:toolbar.removeFavorite")
              : t("explorer:toolbar.addFavorite")
          }
          className={TOOLBAR_OVERFLOW_FAVORITE_CLASS}
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
          <StarIcon className={cn(isCurrentFavorited && "fill-warning/70 text-warning")} />
        </Button>
      </div>

      {/* View: how the listing looks, plus the repository it sits in. */}
      <div className={CLUSTER_CLASS}>
        <ViewMenu disabled={!directory} />
        <FilterMenu disabled={!directory} />
      </div>

      {gitStatus && (
        <div className={cn("shrink-0", TOOLBAR_OVERFLOW_GIT_CLASS)}>
          <GitBranchControl branch={gitStatus.branch} root={gitStatus.root} />
        </div>
      )}

      {/* Panels: trailing edge, so the set stays put as the middle grows and
          shrinks. A toggle fills its chip while on (`aria-pressed`), so an
          open panel is legible at a glance instead of being a guess. */}
      <div className="ml-0.5 flex shrink-0 items-center gap-0.5">
        {onToggleSplit && (
          <Button
            aria-label={
              splitEnabled ? t("explorer:toolbar.closeSplitView") : t("explorer:toolbar.splitView")
            }
            aria-pressed={splitEnabled}
            className={cn(TOOLBAR_OVERFLOW_CLASS, TOGGLE_PRESSED_CLASS)}
            onClick={onToggleSplit}
            size="icon"
            title={
              splitEnabled ? t("explorer:toolbar.closeSplitView") : t("explorer:toolbar.splitView")
            }
            type="button"
            variant="ghost"
          >
            <Columns3Icon />
          </Button>
        )}
        <Button
          aria-label={
            isPreviewOpen
              ? t("explorer:toolbar.collapsePreview")
              : t("explorer:toolbar.expandPreview")
          }
          aria-pressed={isPreviewOpen}
          className={TOGGLE_PRESSED_CLASS}
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
          <EyeIcon />
        </Button>
        <TerminalToggle />
      </div>
    </header>
  );
}
