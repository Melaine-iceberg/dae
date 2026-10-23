/**
 * Small presentational pieces of the explorer's chrome: the listing stats in
 * the path bar, the terminal toggle, and the file-operation progress strip.
 *
 * All of these are self-contained leaf components — each owns only the
 * translation/shortcut lookups it needs — so they were lifted out of
 * `explorer-view.tsx` verbatim.
 */
import { useAtom, useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { LoaderIcon, ProgrammingIcon } from "@solar-icons/react/linear";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Progress } from "@/components/ui/progress";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { useDiskVolumes } from "@/features/sidebar/use-disk-volumes";
import type { DiskVolume } from "@/features/sidebar/types";
import { localeNumber } from "@/i18n/format";
import { cn, formatBytes } from "@/lib/utils";

import type { FileOperationKind, FileOperationProgress } from "./types";
import { explorerStatusAtom } from "./status-strip-atom";
import { GitBranchControl } from "./git-branches";

/**
 * How a toolbar toggle shows that it is on: the chip fills, the icon stays the
 * same. The state already lives on the control as `aria-pressed`, so the
 * styling keys off that attribute and no caller has to remember to paint it.
 */
export const TOGGLE_PRESSED_CLASS = "aria-pressed:bg-accent aria-pressed:text-foreground";

/**
 * Inline listing status for the path bar's trailing edge: how many rows the
 * folder holds, and what a search is doing. It deliberately does not repeat the
 * selection count — that belongs to the action bar, which exists for it.
 *
 * This replaced a dedicated 24px status bar. It belongs beside the breadcrumbs
 * because it describes the folder they name — and because that strip existed
 * only to carry these few facts, removing it returns a whole row of chrome to
 * the listing. Transient activity keeps its own surface: a running file
 * operation still gets `FileOperationStatusBar`, and a Git failure pops up
 * under its own control. Only the always-true facts live here.
 */
export function ListingStats({
  isLoading,
  itemCount,
  searchError,
  searchQuery,
  selectedCount,
  truncated,
}: {
  isLoading: boolean;
  itemCount: number;
  searchError: string | null;
  searchQuery: string | null;
  selectedCount: number;
  truncated: boolean;
}) {
  const { t } = useTranslation("explorer");
  const clearSelectionBinding = formatBinding(useBinding("explorer.clearSelection"));
  // At rest this reports nothing: the status strip owns the folder's row count
  // and its loading state, and a count printed twice (once in the toolbar, once
  // at the foot of the window) is the kind of quiet duplication that makes a
  // dense window feel busy. What stays here is only what the strip cannot say —
  // whether a *search* is what you are looking at, and how to leave a selection.
  const isTransient = Boolean(searchQuery) || Boolean(searchError) || selectedCount > 0;
  if (!isTransient) return null;

  const status = isLoading
    ? searchQuery
      ? t("explorer:listing.searching")
      : t("explorer:listing.loading")
    : searchError
      ? t("explorer:listing.searchFailed")
      : `${t(searchQuery ? "explorer:listing.matchCount" : "explorer:listing.itemCount", {
          count: itemCount,
          display: localeNumber(itemCount),
        })}${truncated ? t("explorer:listing.truncatedSuffix") : ""}`;

  return (
    <span
      aria-live="polite"
      className="ml-auto flex shrink-0 items-center gap-1.5 pl-3 text-micro text-muted-foreground tabular-nums"
    >
      {selectedCount > 0 ? (
        // The selected count is already shown where it matters most — the
        // action bar at the foot of the listing — so repeating it here only
        // spent the breadcrumbs' width on a number they can see elsewhere.
        // What stays is the one thing a selection raises and the action bar
        // does not answer: how to get out of it. Hidden on narrow panes, where
        // the path bar has no room to spare for it.
        <span className="hidden shrink-0 items-center gap-1 @min-[900px]:flex">
          <Kbd className="h-4 px-1 text-nano">{clearSelectionBinding}</Kbd>
          {t("explorer:listing.clearSelectionHint")}
        </span>
      ) : (
        <span className="truncate">{status}</span>
      )}
    </span>
  );
}

/**
 * Integrated-terminal toggle. It moved out of the status bar so it sits with
 * the other view-level controls rather than in a strip of its own.
 */
export function TerminalToggle() {
  const { t } = useTranslation("explorer");
  const [visible, setVisible] = useAtom(terminalVisibleAtom);
  const toggleBinding = formatBinding(useBinding("app.toggleTerminal"));

  return (
    <Button
      aria-label={t("explorer:toolbar.toggleTerminal")}
      aria-pressed={visible}
      className={TOGGLE_PRESSED_CLASS}
      onClick={() => setVisible((open) => !open)}
      size="icon"
      title={t("explorer:toolbar.terminalTitle", { shortcut: toggleBinding })}
      type="button"
      variant="ghost"
    >
      <ProgrammingIcon />
    </Button>
  );
}

export function FileOperationStatusBar({ progress }: { progress: FileOperationProgress }) {
  const { t } = useTranslation("explorer");
  const operationLabel: Record<FileOperationKind, string> = {
    copy: t("explorer:progress.opCopy"),
    move: t("explorer:progress.opMove"),
    delete: t("explorer:progress.opDelete"),
    compress: t("explorer:progress.opCompress"),
    extract: t("explorer:progress.opExtract"),
    properties: t("explorer:progress.opProperties"),
  };
  const total = progress.total;
  const percentage = total && total > 0 ? Math.round((progress.completed / total) * 100) : 0;
  const currentPath = progress.currentPath;
  const operationName = operationLabel[progress.operation];
  const statusText =
    progress.phase === "preparing"
      ? t("explorer:progress.preparing", { op: operationName })
      : progress.phase === "completed"
        ? t("explorer:progress.completed", { op: operationName })
        : t("explorer:progress.inProgress", { op: operationName });

  return (
    <footer
      aria-live="polite"
      className="flex h-status-strip shrink-0 items-center gap-3 px-3"
    >
      <LoaderIcon
        className={cn(
          "size-3.5 shrink-0 text-primary",
          progress.phase !== "completed" && "animate-spin",
        )}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-3 text-micro">
          <span className="truncate">
            {statusText}
            {currentPath ? ` · ${currentPath}` : ""}
          </span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {total === null
              ? t("explorer:progress.counting")
              : t("explorer:progress.counter", {
                  completed: localeNumber(progress.completed),
                  total: localeNumber(total),
                  percentage,
                })}
          </span>
        </div>
        <Progress className="mt-1.5 w-full" size="sm" value={percentage} />
      </div>
    </footer>
  );
}

/** The volume a path lives on: the longest mount point that prefixes it.
 *  Windows mounts are `C:\`, POSIX ones `/`, `/home`, or a network share, so
 *  the longest match is the only one that puts `/home/u` on its own volume
 *  instead of on `/`. */
function pickVolumeFor(volumes: DiskVolume[] | null, path: string | null): DiskVolume | null {
  if (!volumes || !path) return null;
  let best: DiskVolume | null = null;
  for (const volume of volumes) {
    if (!volume.mountPoint) continue;
    const mount = volume.mountPoint.replace(/[\\/]+$/, "");
    if (!mount) continue; // the root volume matches everything below
    const normalized = path.toLowerCase();
    const prefix = mount.toLowerCase();
    if (normalized === prefix || normalized.startsWith(`${prefix}\\`) || normalized.startsWith(`${prefix}/`)) {
      if (!best || mount.length > best.mountPoint.length) best = volume;
    }
  }
  return best;
}

/**
 * The window's status strip: 26px of always-true facts along the bottom of the
 * pane, the way the reference draws it.
 *
 * This shell used to carry one and then dropped it, on the argument that the
 * few facts it held — how many rows, whether a search is running — read better
 * beside the breadcrumbs that name the folder. That argument still holds for
 * transient search/selection status, which stays in the path field. What the
 * strip keeps is the layer the path field cannot: the listing's mode and count,
 * the volume the folder sits on, and the repository branch. Those are facts
 * about the *system* the folder lives in, not about the listing's contents, so
 * they belong at the edge of the window rather than in the toolbar's spine.
 *
 * The Git branch joined them from the toolbar: it is the widest optional chip
 * on a row that must stay one line, and moving it here frees that width for the
 * path and search while keeping the branch one click away.
 *
 * The volume probe is the same hook the rail's disk section uses. Two instances
 * cost one extra `listDisks` a minute and one on focus; the strip is always
 * mounted while the rail's section is collapsed by default, so in practice this
 * is the only reader.
 */
export function ExplorerStatusBar({
  isLoading,
  isSearching,
  itemCount,
  path,
  selectedCount,
  git,
}: {
  isLoading: boolean;
  isSearching: boolean;
  itemCount: number;
  path: string | null;
  selectedCount: number;
  git: { branch: string; root: string } | null;
}) {
  const { t } = useTranslation("explorer");
  const volumes = useDiskVolumes(path);
  const volume = pickVolumeFor(volumes, path);
  const freePercent =
    volume && volume.totalBytes > 0
      ? Math.round((volume.availableBytes / volume.totalBytes) * 100)
      : null;
  const mode = isLoading ? "loading" : isSearching ? "searching" : "ready";

  return (
    <footer
      aria-live="polite"
      className="flex h-status-strip shrink-0 items-center gap-2.5 border-t border-border-soft px-3 text-nano text-muted-foreground select-none"
    >
      {/* The reference's strip reads left to right as `count · selection ·
          filter`, then pushes the volume to the far edge. The green mode pill
          this used to open with was an assertion that nothing had happened
          yet; it now appears only while something *is* happening (reading or
          searching), which is when that fact is worth a 19px capsule. */}
      <span className="truncate tabular-nums">
        {t("explorer:listing.itemCount", {
          count: itemCount,
          display: localeNumber(itemCount),
        })}
      </span>

      {selectedCount > 0 && (
        <>
          <span aria-hidden="true" className="h-3 w-px shrink-0 bg-border" />
          <span className="shrink-0 text-foreground/80 tabular-nums">
            {t("explorer:listing.selectedCount", { display: localeNumber(selectedCount) })}
          </span>
        </>
      )}

      {mode !== "ready" && (
        <span
          className={cn(
            "inline-flex h-[19px] shrink-0 items-center gap-1.5 rounded-full px-2 text-nano",
            "bg-foreground/6 text-muted-foreground",
          )}
        >
          <span aria-hidden="true" className="size-1.5 rounded-full bg-meta" />
          {t(`explorer:listing.mode.${mode}`)}
        </span>
      )}

      {(git || (volume && freePercent !== null)) && (
        <span className="ml-auto flex min-w-0 shrink items-center gap-2.5">
          {/* The branch moves here from the toolbar: it is a property of the
              folder's repository, not an action on the listing, and the strip
              is where the window already keeps system context (the volume).
              Its menu opens upward — see `GitBranchControl`. */}
          {git && <GitBranchControl branch={git.branch} root={git.root} />}
          {volume && freePercent !== null && (
            <span
              className="flex shrink-0 items-center gap-2"
              title={`${volume.mountPoint} · ${volume.fileSystem}`}
            >
              <span
                aria-hidden="true"
                className="block h-[3px] w-16 overflow-hidden rounded-full bg-foreground/10"
              >
                {/* The capsule reports the *used* share, as the reference's does:
                    the bar fills as the volume does. Drawing the free share made
                    a nearly empty disk read as a nearly full bar. */}
                <span
                  className={cn(
                    "block h-full rounded-full",
                    freePercent <= 10 ? "bg-warning" : "bg-foreground/42",
                  )}
                  style={{ width: `${Math.min(Math.max(100 - freePercent, 2), 100)}%` }}
                />
              </span>
              <span className="font-mono tabular-nums">
                {t("explorer:listing.freeSpace", { size: formatBytes(volume.availableBytes) })}
              </span>
            </span>
          )}
        </span>
      )}
    </footer>
  );
}

/**
 * The window's bottom strip, bound to the focused pane.
 *
 * This is the only component that subscribes to `explorerStatusAtom`, and that
 * is the point: the atom is republished whenever a selection, a listing count
 * or a running operation changes, and a subscription in `ExplorerTabs` would
 * re-render the whole window — tab strip, rail and every kept-alive surface —
 * on every click in the listing. Keeping the subscription in this leaf confines
 * the churn to a 26px row.
 *
 * It renders nothing when no folder pane is focused (the Overview and the other
 * workspace surfaces have no status strip of their own).
 */
export function FocusedPaneStatusBar() {
  const status = useAtomValue(explorerStatusAtom);
  if (!status) return null;

  if (status.progress) return <FileOperationStatusBar progress={status.progress} />;

  return (
    <ExplorerStatusBar
      git={status.git}
      isLoading={status.isLoading}
      isSearching={status.isSearching}
      itemCount={status.itemCount}
      path={status.path}
      selectedCount={status.selectedCount}
    />
  );
}
