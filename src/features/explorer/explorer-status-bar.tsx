import { useAtom } from "jotai";
import { ColumnsIcon, ListIcon, RowsIcon, SquaresFourIcon } from "@phosphor-icons/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

import {
  densityAtom,
  viewModeAtom,
  type ExplorerDensity,
  type ExplorerViewMode,
} from "./preferences";

interface ExplorerStatusBarProps {
  itemCount: number;
  isLoading: boolean;
  searchError: string | null;
  searchQuery: string | null;
  selectedCount: number;
  truncated: boolean;
}

export function ExplorerStatusBar({
  itemCount,
  isLoading,
  searchError,
  searchQuery,
  selectedCount,
  truncated,
}: ExplorerStatusBarProps) {
  const countText = isLoading
    ? searchQuery
      ? "正在搜索…"
      : "正在读取…"
    : searchError
      ? "搜索失败"
      : `${itemCount.toLocaleString("zh-CN")} 个${searchQuery ? "匹配项" : "项目"}${truncated ? "（已截断）" : ""}`;

  return (
    <footer
      aria-label="状态栏"
      className="flex h-7 shrink-0 items-center gap-3 border-t bg-background px-3 text-xs text-muted-foreground select-none"
    >
      <span aria-live="polite" className="truncate tabular-nums">
        {countText}
      </span>
      {selectedCount > 0 && (
        <span className="shrink-0 tabular-nums">
          已选择 {selectedCount.toLocaleString("zh-CN")} 项
        </span>
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <DensitySwitcher />
        <div aria-hidden="true" className="mx-1 h-4 w-px bg-border" />
        <ViewModeSwitcher />
      </div>
    </footer>
  );
}

const VIEW_MODE_PRESENTATION = [
  { icon: ListIcon, label: "列表视图", value: "list" },
  { icon: ColumnsIcon, label: "分栏视图", value: "column" },
  { icon: SquaresFourIcon, label: "网格视图", value: "grid" },
] as const satisfies ReadonlyArray<{
  icon: typeof ListIcon;
  label: string;
  value: ExplorerViewMode;
}>;

function ViewModeSwitcher() {
  const [viewMode, setViewMode] = useAtom(viewModeAtom);

  return (
    <div aria-label="视图模式" className="flex items-center gap-0.5" role="group">
      {VIEW_MODE_PRESENTATION.map(({ icon: ModeIcon, label, value: mode }) => (
        <button
          aria-label={label}
          aria-pressed={viewMode === mode}
          className={cn(
            "flex size-5 items-center justify-center rounded-full transition-colors hover:bg-accent hover:text-foreground",
            viewMode === mode && "bg-accent text-foreground",
          )}
          key={mode}
          onClick={() => setViewMode(mode)}
          title={label}
          type="button"
        >
          <ModeIcon size={13} />
        </button>
      ))}
    </div>
  );
}

const DENSITY_PRESENTATION = [
  { label: "紧凑", value: "compact" },
  { label: "舒适", value: "comfortable" },
  { label: "宽松", value: "spacious" },
] as const satisfies ReadonlyArray<{ label: string; value: ExplorerDensity }>;

function DensitySwitcher() {
  const [density, setDensity] = useAtom(densityAtom);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label="显示密度"
        className="flex size-5 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="显示密度"
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
              {item.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
