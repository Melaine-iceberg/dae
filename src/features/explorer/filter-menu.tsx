import { useAtom } from "jotai";
import { FunnelIcon, XIcon } from "@phosphor-icons/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

import {
  DEFAULT_ENTRY_FILTERS,
  entryFiltersAtom,
  hasActiveEntryFilters,
  type ExplorerEntryFilters,
  type ExplorerKindFilter,
  type ExplorerModifiedFilter,
  type ExplorerSizeFilter,
} from "./preferences";

const KIND_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerKindFilter }> = [
  { label: "全部", value: "all" },
  { label: "仅文件夹", value: "folders" },
  { label: "仅文件", value: "files" },
  { label: "仅图片", value: "images" },
];

const MODIFIED_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerModifiedFilter }> = [
  { label: "任意时间", value: "any" },
  { label: "今天", value: "today" },
  { label: "最近 7 天", value: "week" },
  { label: "最近 30 天", value: "month" },
];

const SIZE_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerSizeFilter }> = [
  { label: "任意大小", value: "any" },
  { label: "小于 1 MB", value: "small" },
  { label: "1 MB – 100 MB", value: "medium" },
  { label: "大于 100 MB", value: "large" },
];

/**
 * Toolbar entry-filter menu (SKILL.md §16): kind / modified-time / size
 * buckets applied to the active listing, with one-click reset.
 */
export function FilterMenu({ disabled }: { disabled?: boolean }) {
  const [filters, setFilters] = useAtom(entryFiltersAtom);
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
        aria-label="过滤当前列表"
        className={cn(
          "relative flex size-8 shrink-0 items-center justify-center rounded-full text-muted-foreground outline-none transition-colors hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground disabled:pointer-events-none disabled:opacity-50",
          isActive && "bg-muted text-foreground",
        )}
        disabled={disabled}
        title="过滤当前列表"
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
        <DropdownMenuLabel>类型</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("kind", value as ExplorerKindFilter)}
          value={filters.kind}
        >
          {KIND_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>修改时间</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("modified", value as ExplorerModifiedFilter)}
          value={filters.modified}
        >
          {MODIFIED_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>大小</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => updateFilter("size", value as ExplorerSizeFilter)}
          value={filters.size}
        >
          {SIZE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        {isActive && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuItem
                onSelect={() => setFilters(DEFAULT_ENTRY_FILTERS)}
              >
                <XIcon />
                清除所有过滤器
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
