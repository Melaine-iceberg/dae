import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import type { Icon } from "@solar-icons/react/lib/types";

import {
  CalendarDateIcon,
  CalendarMarkIcon,
  CalendarMinimalisticIcon,
  ChevronsLeftRightIcon,
  ClockCircleIcon,
  CloseIcon,
  DoubleAltArrowLeftIcon,
  DoubleAltArrowRightIcon,
  FileIcon,
  FilterIcon,
  FolderIcon,
  GalleryIcon,
  LayersIcon,
  MoveHorizontalIcon,
} from "@solar-icons/react/line-duotone";

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
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { formatBinding } from "@/features/settings/shortcut-registry";
import { useBinding } from "@/features/settings/settings-atoms";
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

type Option<TValue> = Readonly<{ label: string; value: TValue; icon: Icon }>;

const KIND_OPTIONS: ReadonlyArray<Option<ExplorerKindFilter>> = [
  { label: "filter.kindAll", value: "all", icon: LayersIcon },
  { label: "filter.kindFolders", value: "folders", icon: FolderIcon },
  { label: "filter.kindFiles", value: "files", icon: FileIcon },
  { label: "filter.kindImages", value: "images", icon: GalleryIcon },
];

const MODIFIED_OPTIONS: ReadonlyArray<Option<ExplorerModifiedFilter>> = [
  { label: "filter.modifiedAny", value: "any", icon: ClockCircleIcon },
  { label: "filter.modifiedToday", value: "today", icon: CalendarDateIcon },
  { label: "filter.modifiedWeek", value: "week", icon: CalendarMinimalisticIcon },
  { label: "filter.modifiedMonth", value: "month", icon: CalendarMarkIcon },
];

const SIZE_OPTIONS: ReadonlyArray<Option<ExplorerSizeFilter>> = [
  { label: "filter.sizeAny", value: "any", icon: MoveHorizontalIcon },
  { label: "filter.sizeSmall", value: "small", icon: DoubleAltArrowLeftIcon },
  { label: "filter.sizeMedium", value: "medium", icon: ChevronsLeftRightIcon },
  { label: "filter.sizeLarge", value: "large", icon: DoubleAltArrowRightIcon },
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
  const toggleHiddenBinding = formatBinding(useBinding("explorer.toggleHidden"));

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
          "relative flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground disabled:pointer-events-none disabled:opacity-50",
          isActive && "bg-muted text-foreground",
        )}
        disabled={disabled}
        title={t("filter.ariaLabel")}
      >
        <FilterIcon className="size-4" />
        {isActive && (
          <span
            aria-hidden="true"
            className="absolute top-1 right-1 size-1.5 rounded-full bg-primary"
          />
        )}
      </DropdownMenuTrigger>
      {/* `w-auto`: the anchor is a 28px icon button, so the shared
          anchor-width rule would lock the popup to the 180px floor — 16px
          short of the hidden-files row once its two key chips and the tick
          gutter are paid for, which wrapped the label inside a fixed 28px
          row. The floor still holds the narrow menus up. */}
      <DropdownMenuContent align="end" className="w-auto min-w-menu">
        <DropdownMenuLabel inset>{t("filter.kindLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("kind", value as ExplorerKindFilter)}
          value={filters.kind}
        >
          {KIND_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("filter.modifiedLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("modified", value as ExplorerModifiedFilter)}
          value={filters.modified}
        >
          {MODIFIED_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("filter.sizeLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("size", value as ExplorerSizeFilter)}
          value={filters.size}
        >
          {SIZE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem checked={showHiddenFiles} onCheckedChange={setShowHiddenFiles}>
            {t("filter.showHiddenFiles")}
            {/* Was a hardcoded `Mod` from the platform module, which drifted
                the moment the action was rebound. The chip reads the live
                binding, like every other shortcut hint in the app. */}
            <DropdownMenuShortcut>{toggleHiddenBinding}</DropdownMenuShortcut>
          </DropdownMenuCheckboxItem>
        </DropdownMenuGroup>

        {isActive && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuItem onSelect={() => setFilters(DEFAULT_ENTRY_FILTERS)}>
                <CloseIcon />
                {t("filter.clearAll")}
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
