import { GlobeIcon } from "@phosphor-icons/react";
import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SUPPORTED_LOCALES } from "@/i18n";
import { localeAtom } from "@/i18n/atoms";
import type { AppLocale } from "@/i18n";

/**
 * Sidebar footer language switcher. Mirrors ThemeMenu's dropdown anatomy;
 * the trigger shows a globe regardless of the active locale.
 */
export function LanguageMenu() {
  const { t } = useTranslation();
  const [locale, setLocale] = useAtom(localeAtom);

  const localeName = (value: AppLocale) => t(`common:language.${value}`);
  const title = t("common:language.title", { name: localeName(locale) });

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={title}
        className="flex size-7 items-center justify-center rounded-sm text-muted-foreground transition-colors outline-none hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground"
        title={title}
      >
        <GlobeIcon className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-36">
        <DropdownMenuRadioGroup
          onValueChange={(value) => setLocale(value as AppLocale)}
          value={locale}
        >
          {SUPPORTED_LOCALES.map((value) => (
            <DropdownMenuRadioItem key={value} value={value}>
              {localeName(value)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
