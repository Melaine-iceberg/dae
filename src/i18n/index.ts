import i18next from "i18next";
import type { ResourceKey, ResourceLanguage } from "i18next";
import { initReactI18next } from "react-i18next";

/** Locales shipped with the app. Add a folder under locales/ to extend. */
export const SUPPORTED_LOCALES = ["zh-CN", "en"] as const;
export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

/** Source language; also the fallback when a key is missing. */
export const DEFAULT_LOCALE: AppLocale = "zh-CN";

const STORAGE_KEY = "app.locale";

export function isSupportedLocale(value: unknown): value is AppLocale {
  return typeof value === "string" && (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/** Maps a BCP-47 tag (e.g. `navigator.language`) to a shipped locale. */
export function resolveLocale(tag: string | null | undefined): AppLocale {
  if (!tag) return DEFAULT_LOCALE;
  const lower = tag.toLowerCase();
  if (lower === "zh" || lower.startsWith("zh-") || lower.startsWith("zh_")) return "zh-CN";
  if (lower.startsWith("en")) return "en";
  return DEFAULT_LOCALE;
}

/** First-launch locale: stored preference, else the OS language. */
function initialLocale(): AppLocale {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    // atomWithStorage stores raw strings; unwrap legacy quoted values too.
    const raw = stored ? (stored.startsWith('"') ? JSON.parse(stored) : stored) : null;
    if (isSupportedLocale(raw)) return raw;
  } catch {
    // Ignore malformed storage and fall through to the OS language.
  }
  return resolveLocale(window.navigator.language);
}

// Namespaces are one JSON file per feature, eagerly bundled so translations
// resolve synchronously (initImmediate: false) before the first render.
const localeModules = {
  "zh-CN": import.meta.glob("./locales/zh-CN/*.json", { eager: true }),
  en: import.meta.glob("./locales/en/*.json", { eager: true }),
} as const;

function namespaceOf(path: string): string {
  return path.split("/").pop()!.replace(/\.json$/, "");
}

function bundle(modules: Record<string, unknown>): ResourceLanguage {
  const resources: Record<string, ResourceKey> = {};
  for (const [path, mod] of Object.entries(modules)) {
    resources[namespaceOf(path)] = (mod as { default: ResourceKey }).default;
  }
  return resources;
}

void i18next.use(initReactI18next).init({
  resources: {
    "zh-CN": bundle(localeModules["zh-CN"]),
    en: bundle(localeModules.en),
  },
  lng: initialLocale(),
  fallbackLng: DEFAULT_LOCALE,
  defaultNS: "common",
  supportedLngs: SUPPORTED_LOCALES as unknown as string[],
  interpolation: { escapeValue: false }, // React already escapes output.
  returnNull: false,
});

export { i18next, i18next as i18n };
export { STORAGE_KEY as LOCALE_STORAGE_KEY };
