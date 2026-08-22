import { i18n } from "./index";

/**
 * Locale-aware Intl formatting helpers. Formatters are cached per
 * (locale, options) pair so hot render paths (virtualized file lists) stay
 * allocation-free while still reacting to language changes.
 */

const numberFormatters = new Map<string, Intl.NumberFormat>();
const dateTimeFormatters = new Map<string, Intl.DateTimeFormat>();
const collators = new Map<string, Intl.Collator>();

/** Grouped number per the active UI language, e.g. 1,234 / 1.234. */
export function localeNumber(value: number): string {
  return value.toLocaleString(i18n.language);
}

/** Cached `Intl.NumberFormat` for the active locale. */
export function localeNumberFormat(options?: Intl.NumberFormatOptions): Intl.NumberFormat {
  const key = `${i18n.language}|${options ? JSON.stringify(options) : ""}`;
  let formatter = numberFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(i18n.language, options);
    numberFormatters.set(key, formatter);
  }
  return formatter;
}

/** Cached `Intl.DateTimeFormat` for the active locale. */
export function localeDateTimeFormat(options?: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${i18n.language}|${options ? JSON.stringify(options) : ""}`;
  let formatter = dateTimeFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(i18n.language, options);
    dateTimeFormatters.set(key, formatter);
  }
  return formatter;
}

/** Cached natural-order collator for the active locale (file-name sorting). */
export function localeCollator(options?: Intl.CollatorOptions): Intl.Collator {
  const key = `${i18n.language}|${options ? JSON.stringify(options) : ""}`;
  let collator = collators.get(key);
  if (!collator) {
    collator = new Intl.Collator(i18n.language, options);
    collators.set(key, collator);
  }
  return collator;
}
