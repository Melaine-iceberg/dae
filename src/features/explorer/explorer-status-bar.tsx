import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  ColumnsIcon,
  ListIcon,
  RowsIcon,
  SquaresFourIcon,
  TerminalIcon,
} from "@phosphor-icons/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { localeNumber } from "@/i18n/format";
import { cn } from "@/lib/utils";

import { StatusBarGit } from "./git-branches";
import {
  densityAtom,
  viewModeAtom,
  type ExplorerDensity,
  type ExplorerViewMode,
} from "./preferences";

interface ExplorerStatusBarProps {
  gitBranch?: string | null;
  gitRoot?: string | null;
  itemCount: number;
  isLoading: boolean;
  searchError: string | null;
  searchQuery: string | null;
  selectedCount: number;
  truncated: boolean;
}

export function ExplorerStatusBar({
  gitBranch,
  gitRoot,
  itemCount,
  isLoading,
  searchError,
  searchQuery,
  selectedCount,
  truncated,
}: ExplorerStatusBarProps) {
  const { t } = useTranslation("explorer");
  const countText = isLoading
    ? searchQuery
      ? t("statusBar.searching")
      : t("statusBar.loading")
    : searchError
      ? t("statusBar.searchFailed")
      : `${t(searchQuery ? "statusBar.matchCount" : "statusBar.itemCount", {
          count: itemCount,
          display: localeNumber(itemCount),
        })}${truncated ? t("statusBar.truncatedSuffix") : ""}`;

  return (
    <footer
      aria-label={t("statusBar.ariaLabel")}
      className="flex h-7 shrink-0 items-center gap-3 border-t border-border/60 bg-muted/50 px-3 text-xs text-muted-foreground select-none"
    >
      <span aria-live="polite" className="truncate tabular-nums">
        {countText}
      </span>
      {selectedCount > 0 && (
        <span className="shrink-0 tabular-nums">
          {t("statusBar.selectedCount", { number: localeNumber(selectedCount) })}
        </span>
      )}
      <StatusBarGit branch={gitBranch ?? null} root={gitRoot ?? null} />
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <TerminalToggle />
        <div aria-hidden="true" className="mx-1 h-4 w-px bg-border" />
        <DensitySwitcher />
        <div aria-hidden="true" className="mx-1 h-4 w-px bg-border" />
        <ViewModeSwitcher />
      </div>
    </footer>
  );
}

function TerminalToggle() {
  const { t } = useTranslation("explorer");
  const [visible, setVisible] = useAtom(terminalVisibleAtom);

  return (
    <button
      aria-label={t("statusBar.toggleTerminal")}
      aria-pressed={visible}
      className={cn(
        "flex size-5 items-center justify-center rounded-xs transition-colors hover:bg-accent hover:text-foreground",
        visible && "bg-accent text-foreground",
      )}
      onClick={() => setVisible((open) => !open)}
      title={t("statusBar.terminalTitle")}
      type="button"
    >
      <TerminalIcon size={13} />
    </button>
  );
}

const VIEW_MODE_PRESENTATION = [
  { icon: ListIcon, label: "statusBar.viewList", value: "list" },
  { icon: ColumnsIcon, label: "statusBar.viewColumn", value: "column" },
  { icon: SquaresFourIcon, label: "statusBar.viewGrid", value: "grid" },
] as const satisfies ReadonlyArray<{
  icon: typeof ListIcon;
  label: string;
  value: ExplorerViewMode;
}>;

function ViewModeSwitcher() {
  const { t } = useTranslation("explorer");
  const [viewMode, setViewMode] = useAtom(viewModeAtom);

  // Segmented control: a tonal track with a raised card for the active mode,
  // so the current view reads at a glance (macOS / Windows 11 convention).
  return (
    <div
      aria-label={t("statusBar.viewModeLabel")}
      className="flex items-center gap-0.5 rounded-md bg-muted p-0.5"
      role="group"
    >
      {VIEW_MODE_PRESENTATION.map(({ icon: ModeIcon, label, value: mode }) => (
        <button
          aria-label={t(label)}
          aria-pressed={viewMode === mode}
          className={cn(
            "flex h-[18px] w-6 items-center justify-center rounded-[5px] text-muted-foreground transition-[background-color,color,box-shadow] duration-fast hover:text-foreground",
            viewMode === mode && "bg-card text-primary shadow-ambient-xs",
          )}
          key={mode}
          onClick={() => setViewMode(mode)}
          title={t(label)}
          type="button"
        >
          <ModeIcon size={12} />
        </button>
      ))}
    </div>
  );
}

const DENSITY_PRESENTATION = [
  { label: "statusBar.densityCompact", value: "compact" },
  { label: "statusBar.densityComfortable", value: "comfortable" },
  { label: "statusBar.densitySpacious", value: "spacious" },
] as const satisfies ReadonlyArray<{ label: string; value: ExplorerDensity }>;

function DensitySwitcher() {
  const { t } = useTranslation("explorer");
  const [density, setDensity] = useAtom(densityAtom);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("statusBar.densityLabel")}
        className="flex size-5 items-center justify-center rounded-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title={t("statusBar.densityLabel")}
      >
        <RowsIcon size={13} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          onValueChange={(value) => setDensity(value as ExplorerDensity)}
          value={density}
        >
          {DENSITY_PRESENTATION.map((item) => (
            <DropdownMenuRadioItem key={item.value} value={item.value}>
              {t(item.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
