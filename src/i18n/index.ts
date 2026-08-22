import i18next from "i18next";
import type { Resource, ResourceKey, ResourceLanguage } from "i18next";
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

// Namespaces are one JSON file per feature. Only the default locale ships in
// the entry chunk; other locales load on demand (see ensureLocaleLoaded) so
// their JSON is never fetched or parsed by users who never switch language.
const zhModules = import.meta.glob("./locales/zh-CN/*.json", { eager: true });
const enModules = import.meta.glob("./locales/en/*.json");

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

async function loadLocale(locale: AppLocale): Promise<ResourceLanguage> {
  if (locale === DEFAULT_LOCALE) return bundle(zhModules);
  const entries = await Promise.all(
    Object.entries(enModules).map(async ([path, load]) => {
      const mod = (await load()) as { default: ResourceKey };
      return [namespaceOf(path), mod.default] as const;
    }),
  );
  return Object.fromEntries(entries);
}

/** Registers a lazily shipped locale with i18next (no-op if already loaded). */
export async function ensureLocaleLoaded(locale: AppLocale): Promise<void> {
  if (i18next.hasResourceBundle(locale, "common")) return;
  const language = await loadLocale(locale);
  for (const [ns, resources] of Object.entries(language)) {
    i18next.addResourceBundle(locale, ns, resources);
  }
}

const startupLocale = initialLocale();

/** Resolves once i18next has initialized with the startup locale's resources. */
export const i18nReady: Promise<unknown> = (async () => {
  const resources: Resource = { [DEFAULT_LOCALE]: bundle(zhModules) };
  if (startupLocale !== DEFAULT_LOCALE) {
    resources[startupLocale] = await loadLocale(startupLocale);
  }
  return i18next.use(initReactI18next).init({
    resources,
    lng: startupLocale,
    fallbackLng: DEFAULT_LOCALE,
    defaultNS: "common",
    supportedLngs: SUPPORTED_LOCALES as unknown as string[],
    // Non-default locales arrive via addResourceBundle when switched to.
    partialBundledLanguages: true,
    interpolation: { escapeValue: false }, // React already escapes output.
    returnNull: false,
  });
})();

export { i18next, i18next as i18n, initialLocale };
export { STORAGE_KEY as LOCALE_STORAGE_KEY };
