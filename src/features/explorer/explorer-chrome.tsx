/**
 * Small presentational pieces of the explorer's chrome: the toolbar's
 * separators, the listing stats in the path bar, the terminal toggle, and the
 * file-operation progress strip.
 *
 * All of these are self-contained leaf components — each owns only the
 * translation/shortcut lookups it needs — so they were lifted out of
 * `explorer-view.tsx` verbatim.
 */
import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { LoaderCircle, SquareTerminal } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Progress } from "@/components/ui/progress";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { localeNumber } from "@/i18n/format";
import { cn } from "@/lib/utils";

import type { FileOperationKind, FileOperationProgress } from "./types";

/**
 * Hairline between toolbar groups. `className` exists so a group can take its
 * separator down with it on narrow windows — a lone divider with nothing on
 * one side reads as a rendering bug.
 */
export function ToolbarSeparator({ className }: { className?: string }) {
  return <div aria-hidden="true" className={cn("mx-0.5 h-4 w-px bg-border", className)} />;
}

/**
 * Inline listing status for the path bar's trailing edge: how many rows the
 * folder holds, what is selected, and what a search is doing.
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
        // While a selection exists it is the salient fact, and the bar is
        // narrow enough that chip + hint + item count together squeezed the
        // breadcrumbs into a sub-10px box whose text then painted across the
        // chip. Dropping the (still visible in the empty selection state, and
        // moot across a navigation that clears the selection) total reclaims
        // the room the crumbs need.
        <>
          <span className="rounded-xs bg-selection px-1.5 text-foreground">
            {t("explorer:listing.selectedCount", { display: localeNumber(selectedCount) })}
          </span>
          {/* A selection is the one state where "how do I get out of this"
              is a real question, so the way out is spelled out here instead of
              being left to the keyboard. Hidden on narrow panes, where the
              path bar has no room to spare for it. */}
          <span className="hidden shrink-0 items-center gap-1 min-[840px]:flex">
            <Kbd className="h-4 px-1 text-nano">{clearSelectionBinding}</Kbd>
            {t("explorer:listing.clearSelectionHint")}
          </span>
        </>
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
      className={cn(visible && "bg-accent text-foreground")}
      onClick={() => setVisible((open) => !open)}
      size="icon"
      title={t("explorer:toolbar.terminalTitle", { shortcut: toggleBinding })}
      type="button"
      variant="ghost"
    >
      <SquareTerminal />
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
      className="flex h-status-strip shrink-0 items-center gap-3 border-t border-border bg-card px-3"
    >
      <LoaderCircle
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
