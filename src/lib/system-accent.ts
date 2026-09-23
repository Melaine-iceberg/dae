/**
 * The system-accent seam.
 *
 * The shell's accent is meant to be the OS accent — the one colour Windows,
 * macOS and every Linux desktop already lets the user pick — because that is
 * what makes a single cross-platform app read as native on all three without
 * imitating any of them. It is also the only place this design system allows
 * a saturated colour that is not carrying meaning.
 *
 * Reading that colour is platform work and is deliberately NOT done here:
 *
 *   Windows   HKCU\Software\Microsoft\Windows\DWM\AccentColor (ARGB)
 *   macOS     NSColor.controlAccentColor (needs a native call)
 *   Linux     org.freedesktop.appearance accent-color via the settings
 *             portal, falling back to the GTK theme's accent
 *
 * This module is the seam instead. It owns the shape of the answer and the
 * CSS contract, so the platform reader has exactly one function to replace
 * and one call site to leave alone, and can land on its own schedule.
 *
 * The contract, in full — mirror `lib/theme.ts`, which is why these two are
 * named as a pair:
 *
 *   useEffect(() => watchSystemAccent(applySystemAccent), []);   // App.tsx
 *
 * `applySystemAccent` is implemented and is the whole write side. It writes
 * two custom properties on <html>; everything accent-coloured in App.css
 * derives from them and falls back to the shipped indigo until it has run
 * once. `watchSystemAccent` is the read side and is a stub — it is where a
 * `commands.getSystemAccent()` plus a change event goes, and replacing its
 * body is the entire job. Its argument is already `applySystemAccent`, so no
 * call site changes when it starts working.
 *
 * Two properties, not one. A user-picked accent can be any lightness, and no
 * CSS-only contrast test is portable enough to derive a label colour from it
 * (`color-contrast()` is Safari-only), so the ink is chosen here from the
 * hue's relative luminance. A bridge normally supplies only the hue; pass an
 * ink explicitly when the platform already guarantees one (macOS ships
 * labelColor resolved against controlAccentColor).
 */

/** Written on `<html>`; the accent hue, any CSS `<color>`. */
export const SYSTEM_ACCENT_PROPERTY = "--system-accent";

/** Written on `<html>`; a label colour that clears ≈4.5:1 on the hue. */
export const SYSTEM_ACCENT_INK_PROPERTY = "--system-accent-ink";

/** The two inks `applySystemAccent` picks between, by luminance. */
const INK_LIGHT = "#ffffff";
const INK_DARK = "#101116";

/**
 * Normalises any CSS colour to 8-bit sRGB.
 *
 * A canvas is the only portable CSS-colour parser: it accepts hex, `rgb()`,
 * `oklch()`, `color-mix()` and named colours alike and resolves them through
 * the same code path the renderer uses. `getComputedStyle` on a detached span
 * is the obvious alternative and does not resolve on a detached node in every
 * engine, so it is not the alternative.
 */
function toSrgb(color: string): { b: number; g: number; r: number } | null {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return null;

  context.fillStyle = "#000";
  context.fillRect(0, 0, 1, 1);
  context.fillStyle = color;
  // An unparseable value is rejected and leaves the previous fill in place,
  // so painting black first turns "did it parse?" into a comparison.
  if (context.fillStyle === "#000000" && color.toLowerCase() !== "#000") return null;

  context.fillRect(0, 0, 1, 1);
  const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
  return { r, g, b };
}

/** WCAG 2.x relative luminance of an sRGB colour. */
function relativeLuminance({ b, g, r }: { b: number; g: number; r: number }): number {
  const channel = (value: number) => {
    const srgb = value / 255;
    return srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG 2.x contrast ratio between two relative luminances. */
function contrastRatio(a: number, b: number): number {
  const [lighter, darker] = a > b ? [a, b] : [b, a];
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * Chooses the ink that reads better on `accent`, and reports how well it
 * does. 4.5 is AA for body text; the shell only sets these inks at 13px and
 * above, which is what AA body text already covers.
 */
function resolveInk(accent: string): { contrast: number; ink: string } {
  const luminance = relativeLuminance(toSrgb(accent) ?? { r: 70, g: 73, b: 201 });
  const lightContrast = contrastRatio(luminance, relativeLuminance({ r: 255, g: 255, b: 255 }));
  const darkContrast = contrastRatio(luminance, relativeLuminance({ r: 16, g: 17, b: 22 }));
  return lightContrast >= darkContrast
    ? { contrast: lightContrast, ink: INK_LIGHT }
    : { contrast: darkContrast, ink: INK_DARK };
}/**
 * Retints the shell to `accent`, or hands it back to the shipped indigo when
 * given `null`.
 *
 * Rejects anything that is not a parseable colour rather than writing it:
 * `--primary` resolves through these properties, so one bad value would leave
 * every filled button, focus ring and selection without a colour at all. The
 * window keeps its current tint on rejection, which is the failure that hurts
 * least.
 *
 * The ink is derived from `accent` unless one is supplied.
 */
export function applySystemAccent(accent: string | null, ink?: string): void {
  const root = document.documentElement;
  if (accent === null) {
    root.style.removeProperty(SYSTEM_ACCENT_PROPERTY);
    root.style.removeProperty(SYSTEM_ACCENT_INK_PROPERTY);
    window.dispatchEvent(new CustomEvent("app-system-accent-change"));
    return;
  }

  if (!toSrgb(accent)) {
    console.warn(`[system-accent] ignoring unparseable accent ${JSON.stringify(accent)}`);
    return;
  }

  const resolved = resolveInk(accent);
  // A user accent may simply be too pale (or too dark) for either ink. Say so
  // rather than shipping white-on-white: this is a warning and not a rejection
  // because the same accent is also correct on surfaces where it is never used
  // as a fill, and the platform is the authority on what the user picked.
  if (ink === undefined && resolved.contrast < 4.5) {
    console.warn(
      `[system-accent] ${accent} carries only ${resolved.contrast.toFixed(2)}:1 against its ` +
        `best ink (${resolved.ink}); filled buttons will be under AA. Supply an ink explicitly.`,
    );
  }

  root.style.setProperty(SYSTEM_ACCENT_PROPERTY, accent);
  root.style.setProperty(SYSTEM_ACCENT_INK_PROPERTY, ink ?? resolved.ink);
  window.dispatchEvent(new CustomEvent("app-system-accent-change"));
}
/**
 * The accent currently in force, or `null` while the shell is on its shipped
 * default. For surfaces that need to know which way they went — the terminal
 * palette hands xterm plain hex values, so it cannot follow a CSS variable
 * and has to be told.
 */
export function getSystemAccent(): string | null {
  return document.documentElement.style.getPropertyValue(SYSTEM_ACCENT_PROPERTY) || null;
}

/**
 * Follows the OS accent and hands each reading to `onChange`.
 *
 * THE STUB. This is the only thing left to implement, and the shape of it is
 * fixed so that implementing it changes nothing else:
 *
 *   1. read the platform accent into a CSS colour string (see the platform
 *      notes at the top of this file);
 *   2. call `onChange(colour)`, and again whenever the platform reports a
 *      change — Windows posts WM_DWMCOLORIZATIONCOLORCHANGED, macOS KVO on
 *      NSUserDefaults AppleAccentColor, Linux a portal SettingsChanged;
 *   3. return a disposer that stops listening.
 *
 * Pass `null` through `onChange` when the platform has no accent setting, and
 * `applySystemAccent` will fall back to the shipped indigo. Today it does
 * nothing at all, which is why the shell looks the way it always has.
 */
export function watchSystemAccent(onChange: (accent: string | null) => void): () => void {
  void onChange;
  return () => {};
}
