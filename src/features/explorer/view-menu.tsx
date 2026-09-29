import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  Calendar,
  Columns3,
  HardDrive,
  LayoutGrid,
  List,
  Monitor,
  Palette,
  Rows2,
  Rows3,
  Rows4,
  Shapes,
  SlidersHorizontal,
  Type,
  type LucideIcon,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import {
  DEFAULT_SORT_ORDER,
  densityAtom,
  foldersFirstAtom,
  iconStyleAtom,
  sortKeyAtom,
  sortOrderAtom,
  viewModeAtom,
  type ExplorerDensity,
  type ExplorerIconStyle,
  type ExplorerSortKey,
  type ExplorerSortOrder,
  type ExplorerViewMode,
} from "./preferences";

type Option<TValue> = Readonly<{ label: string; value: TValue; icon: LucideIcon }>;

const VIEW_MODE_OPTIONS: ReadonlyArray<Option<ExplorerViewMode>> = [
  { label: "view.modeList", value: "list", icon: List },
  { label: "view.modeColumn", value: "column", icon: Columns3 },
  { label: "view.modeGrid", value: "grid", icon: LayoutGrid },
];

const DENSITY_OPTIONS: ReadonlyArray<Option<ExplorerDensity>> = [
  { label: "view.densityCompact", value: "compact", icon: Rows4 },
  { label: "view.densityComfortable", value: "comfortable", icon: Rows3 },
  { label: "view.densitySpacious", value: "spacious", icon: Rows2 },
];

const ICON_STYLE_OPTIONS: ReadonlyArray<Option<ExplorerIconStyle>> = [
  { label: "view.iconSystem", value: "system", icon: Monitor },
  { label: "view.iconThemed", value: "themed", icon: Palette },
];

const SORT_KEY_OPTIONS: ReadonlyArray<Option<ExplorerSortKey>> = [
  { label: "sort.keyName", value: "name", icon: Type },
  { label: "sort.keySize", value: "size", icon: HardDrive },
  { label: "sort.keyModified", value: "modified", icon: Calendar },
  { label: "sort.keyType", value: "type", icon: Shapes },
];

const SORT_ORDER_OPTIONS: ReadonlyArray<Option<ExplorerSortOrder>> = [
  { label: "sort.orderAscending", value: "asc", icon: ArrowUpNarrowWide },
  { label: "sort.orderDescending", value: "desc", icon: ArrowDownWideNarrow },
];

/**
 * The toolbar's single display menu: how the listing is arranged (view mode,
 * density, icon set, sort, folder pinning).
 *
 * These used to be split across three surfaces — a view-mode segmented control
 * and a density dropdown in the status bar, plus a sort dropdown in the
 * toolbar — which meant one question ("how is this list arranged?") had three
 * separate answers in three separate places, one of them a permanently
 * 24px-tall strip. Folding them into one grouped menu lets the status bar go
 * away entirely and matches how Linear answers the same question, with labelled
 * sections in a single popover.
 *
 * Entry *filtering* deliberately stays in its own menu: it changes which rows
 * exist rather than how they are shown, and it carries an active-state dot that
 * earns its own affordance.
 */
export function ViewMenu({ disabled }: { disabled?: boolean }) {
  const { t } = useTranslation("explorer");
  const [viewMode, setViewMode] = useAtom(viewModeAtom);
  const [density, setDensity] = useAtom(densityAtom);
  const [iconStyle, setIconStyle] = useAtom(iconStyleAtom);
  const [sortKey, setSortKey] = useAtom(sortKeyAtom);
  const [sortOrder, setSortOrder] = useAtom(sortOrderAtom);
  const [foldersFirst, setFoldersFirst] = useAtom(foldersFirstAtom);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("view.menuLabel")}
        className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground disabled:pointer-events-none disabled:opacity-50"
        disabled={disabled}
        title={t("view.menuLabel")}
      >
        <SlidersHorizontal className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-48">
        <DropdownMenuLabel inset>{t("view.modeLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setViewMode(value as ExplorerViewMode)}
          value={viewMode}
        >
          {VIEW_MODE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("view.densityLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setDensity(value as ExplorerDensity)}
          value={density}
        >
          {DENSITY_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("view.iconStyleLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setIconStyle(value as ExplorerIconStyle)}
          value={iconStyle}
        >
          {ICON_STYLE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("sort.sortBy")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => {
            const key = value as ExplorerSortKey;
            setSortKey(key);
            // Switching keys restarts at the key's default direction, matching
            // list header behavior.
            setSortOrder(DEFAULT_SORT_ORDER[key]);
          }}
          value={sortKey}
        >
          {SORT_KEY_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel inset>{t("sort.direction")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setSortOrder(value as ExplorerSortOrder)}
          value={sortOrder}
        >
          {SORT_ORDER_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value} icon={option.icon}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem checked={foldersFirst} onCheckedChange={setFoldersFirst}>
            {t("sort.foldersFirst")}
          </DropdownMenuCheckboxItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
