import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { FunnelIcon, XIcon } from "@phosphor-icons/react";

import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MOD_KEY } from "@/lib/platform";
import { cn } from "@/lib/utils";

import {
  DEFAULT_ENTRY_FILTERS,
  entryFiltersAtom,
  hasActiveEntryFilters,
  showHiddenFilesAtom,
  type ExplorerEntryFilters,
  type ExplorerKindFilter,
  type ExplorerModifiedFilter,
  type ExplorerSizeFilter,
} from "./preferences";

const KIND_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerKindFilter }> = [
  { label: "filter.kindAll", value: "all" },
  { label: "filter.kindFolders", value: "folders" },
  { label: "filter.kindFiles", value: "files" },
  { label: "filter.kindImages", value: "images" },
];

const MODIFIED_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerModifiedFilter }> = [
  { label: "filter.modifiedAny", value: "any" },
  { label: "filter.modifiedToday", value: "today" },
  { label: "filter.modifiedWeek", value: "week" },
  { label: "filter.modifiedMonth", value: "month" },
];

const SIZE_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerSizeFilter }> = [
  { label: "filter.sizeAny", value: "any" },
  { label: "filter.sizeSmall", value: "small" },
  { label: "filter.sizeMedium", value: "medium" },
  { label: "filter.sizeLarge", value: "large" },
];

/**
 * Toolbar entry-filter menu (SKILL.md §16): kind / modified-time / size
 * buckets applied to the active listing, with one-click reset.
 */
export function FilterMenu({ disabled }: { disabled?: boolean }) {
  const { t } = useTranslation("explorer");
  const [filters, setFilters] = useAtom(entryFiltersAtom);
  const [showHiddenFiles, setShowHiddenFiles] = useAtom(showHiddenFilesAtom);
  const isActive = hasActiveEntryFilters(filters);

  const updateFilter = <TKey extends keyof ExplorerEntryFilters>(
    key: TKey,
    value: ExplorerEntryFilters[TKey],
  ) => {
    setFilters({ ...filters, [key]: value });
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("filter.ariaLabel")}
        className={cn(
          "relative flex size-8 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none transition-colors hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground disabled:pointer-events-none disabled:opacity-50",
          isActive && "bg-muted text-foreground",
        )}
        disabled={disabled}
        title={t("filter.ariaLabel")}
      >
        <FunnelIcon />
        {isActive && (
          <span
            aria-hidden="true"
            className="absolute top-1 right-1 size-1.5 rounded-full bg-primary"
          />
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-44">
        <DropdownMenuLabel>{t("filter.kindLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("kind", value as ExplorerKindFilter)}
          value={filters.kind}
        >
          {KIND_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("filter.modifiedLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("modified", value as ExplorerModifiedFilter)}
          value={filters.modified}
        >
          {MODIFIED_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("filter.sizeLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("size", value as ExplorerSizeFilter)}
          value={filters.size}
        >
          {SIZE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem checked={showHiddenFiles} onCheckedChange={setShowHiddenFiles}>
            {t("filter.showHiddenFiles", { modifier: MOD_KEY })}
          </DropdownMenuCheckboxItem>
        </DropdownMenuGroup>

        {isActive && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuItem onSelect={() => setFilters(DEFAULT_ENTRY_FILTERS)}>
                <XIcon />
                {t("filter.clearAll")}
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
