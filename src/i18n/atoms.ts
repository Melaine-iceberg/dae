import { useEffect } from "react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { atomWithStorage } from "jotai/utils";

import { commands } from "@/bindings";
import { DEFAULT_LOCALE, i18n, isSupportedLocale } from "./index";
import type { AppLocale } from "./index";

/**
 * The persisted UI language. `getOnInit` mirrors what i18next already
 * resolved at startup so the atom and the instance never disagree.
 */
export const localeAtom = atomWithStorage<AppLocale>(
  "app.locale",
  isSupportedLocale(i18n.language) ? (i18n.language as AppLocale) : DEFAULT_LOCALE,
  undefined,
  { getOnInit: true },
);

/** Keeps i18next (and `<html lang>`) aligned with the locale atom, and
 *  pushes the locale's duplicate-name token to the backend. */
export function useLocaleSync(): void {
  const locale = useAtomValue(localeAtom);
  const { t } = useTranslation("common");

  useEffect(() => {
    if (i18n.language !== locale) void i18n.changeLanguage(locale);
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    void commands.setDuplicateSuffix(t("duplicateSuffix")).catch(() => {
      // The command only fails outside Tauri (plain-browser dev); the
      // backend keeps its default suffix there.
    });
  }, [t]);
}

/** Convenience for non-React modules: revalidate on language change. */
export function useT() {
  return useTranslation().t;
}
