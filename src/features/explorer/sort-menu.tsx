import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { ArrowsDownUpIcon } from "@phosphor-icons/react";

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
import { cn } from "@/lib/utils";

import {
  DEFAULT_SORT_ORDER,
  foldersFirstAtom,
  sortKeyAtom,
  sortOrderAtom,
  type ExplorerSortKey,
  type ExplorerSortOrder,
} from "./preferences";

const SORT_KEY_OPTIONS: ReadonlyArray<{
  key: ExplorerSortKey;
  label: string;
}> = [
  { key: "name", label: "sort.keyName" },
  { key: "size", label: "sort.keySize" },
  { key: "modified", label: "sort.keyModified" },
  { key: "type", label: "sort.keyType" },
];

const SORT_ORDER_OPTIONS: ReadonlyArray<{
  label: string;
  order: ExplorerSortOrder;
}> = [
  { label: "sort.orderAscending", order: "asc" },
  { label: "sort.orderDescending", order: "desc" },
];

/**
 * Toolbar sort menu: pick the sort key (name / size / modified / type),
 * direction, and whether folders stay pinned ahead of files. Available in
 * every view mode, complementing the list-view column headers.
 */
export function SortMenu({ disabled }: { disabled?: boolean }) {
  const { t } = useTranslation("explorer");
  const [sortKey, setSortKey] = useAtom(sortKeyAtom);
  const [sortOrder, setSortOrder] = useAtom(sortOrderAtom);
  const [foldersFirst, setFoldersFirst] = useAtom(foldersFirstAtom);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("sort.menuLabel")}
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none transition-colors hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground disabled:pointer-events-none disabled:opacity-50",
        )}
        disabled={disabled}
        title={t("sort.menuLabel")}
      >
        <ArrowsDownUpIcon />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-44">
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
            <DropdownMenuRadioItem key={option.key} value={option.key}>
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
            <DropdownMenuRadioItem key={option.order} value={option.order}>
              {t(option.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem
            checked={foldersFirst}
            onCheckedChange={setFoldersFirst}
          >
            {t("sort.foldersFirst")}
          </DropdownMenuCheckboxItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
