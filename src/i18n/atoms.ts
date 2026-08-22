import { useEffect } from "react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { atomWithStorage } from "jotai/utils";

import { commands } from "@/bindings";
import { ensureLocaleLoaded, i18n, i18nReady, initialLocale } from "./index";
import type { AppLocale } from "./index";

/**
 * The persisted UI language. The fallback repeats the startup resolution
 * (stored preference, else OS language) so the atom never depends on
 * i18next's initialization order.
 */
export const localeAtom = atomWithStorage<AppLocale>("app.locale", initialLocale(), undefined, {
  getOnInit: true,
});

/** Keeps i18next (and `<html lang>`) aligned with the locale atom, and
 *  pushes the locale's duplicate-name token to the backend. */
export function useLocaleSync(): void {
  const locale = useAtomValue(localeAtom);
  const { t } = useTranslation("common");

  useEffect(() => {
    void i18nReady.then(async () => {
      if (i18n.language !== locale) {
        // Non-default locales ship as lazy chunks; register before switching.
        await ensureLocaleLoaded(locale);
        await i18n.changeLanguage(locale);
      }
      document.documentElement.lang = locale;
    });
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
