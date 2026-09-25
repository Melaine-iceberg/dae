import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { SlidersHorizontal } from "lucide-react";

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

const VIEW_MODE_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerViewMode }> = [
  { label: "view.modeList", value: "list" },
  { label: "view.modeColumn", value: "column" },
  { label: "view.modeGrid", value: "grid" },
];

const DENSITY_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerDensity }> = [
  { label: "view.densityCompact", value: "compact" },
  { label: "view.densityComfortable", value: "comfortable" },
  { label: "view.densitySpacious", value: "spacious" },
];

const ICON_STYLE_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerIconStyle }> = [
  { label: "view.iconSystem", value: "system" },
  { label: "view.iconThemed", value: "themed" },
];

const SORT_KEY_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerSortKey }> = [
  { label: "sort.keyName", value: "name" },
  { label: "sort.keySize", value: "size" },
  { label: "sort.keyModified", value: "modified" },
  { label: "sort.keyType", value: "type" },
];

const SORT_ORDER_OPTIONS: ReadonlyArray<{ label: string; value: ExplorerSortOrder }> = [
  { label: "sort.orderAscending", value: "asc" },
  { label: "sort.orderDescending", value: "desc" },
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
        <DropdownMenuLabel>{t("view.modeLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setViewMode(value as ExplorerViewMode)}
          value={viewMode}
        >
          {VIEW_MODE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("view.densityLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setDensity(value as ExplorerDensity)}
          value={density}
        >
          {DENSITY_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("view.iconStyleLabel")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setIconStyle(value as ExplorerIconStyle)}
          value={iconStyle}
        >
          {ICON_STYLE_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("sort.sortBy")}</DropdownMenuLabel>
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
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("sort.direction")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => setSortOrder(value as ExplorerSortOrder)}
          value={sortOrder}
        >
          {SORT_ORDER_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
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
