/**
 * Curated 16-color ANSI palettes for the integrated terminal.
 *
 * xterm's built-in ANSI colors are saturated primaries designed for a pure
 * black background; on the app's surfaces (`--card` is `#101112` dark,
 * `#ffffff` light) they are harsh and, in the light theme, several are outright
 * unreadable. These palettes are sampled from the house color tokens in
 * `App.css` and tuned per theme so `ls --color`, git diffs and friends are
 * legible in both modes.
 *
 * Slot order is the canonical ANSI indexing (0-7 normal, 8-15 bright):
 *   0 black, 1 red, 2 green, 3 yellow, 4 blue, 5 magenta, 6 cyan, 7 white,
 *   8 brightBlack … 15 brightWhite.
 *
 * A complete user override (`settings.terminal.ansiColors`, exactly 16 entries
 * — the backend drops any other length) wins over the theme-derived palette.
 */

/** The 16 ANSI slots in canonical order. */
export type AnsiPalette = readonly string[];

/** Dark, tuned to `--card: #101112` / `--foreground: #f7f8f8`. */
export const DARK_ANSI: AnsiPalette = Object.freeze([
  "#26282c", // black — the border tone, never pure #000 (too harsh on graphite)
  "#ff6b72", // red — --destructive
  "#63d398", // green — --success
  "#e8a33d", // yellow — warm amber (--warning)
  "#8284f8", // blue — --primary, already light enough for the dark island
  "#c792ea", // magenta
  "#5acfd9", // cyan — --tone-cyan
  "#f7f8f8", // white — --foreground
  "#8a8f98", // brightBlack — --muted-foreground (dim text stays legible)
  "#ff8a8a", // brightRed
  "#8ce0b3", // brightGreen
  "#ffc868", // brightYellow — warm amber pastel
  "#8fb2ff", // brightBlue
  "#f08fce", // brightMagenta
  "#8ce0e6", // brightCyan
  "#ffffff", // brightWhite
]);

/** Light, tuned to `--card: #ffffff` / `--foreground: #282a30`. */
export const LIGHT_ANSI: AnsiPalette = Object.freeze([
  "#282a30", // black — soft near-black (--foreground), not pure #000
  "#c22f2f", // red — --destructive
  "#17804a", // green — --success
  "#8a6318", // yellow — darkened amber so it reads on white
  "#4649c9", // blue — --primary
  "#b8438a", // magenta
  "#0d848c", // cyan — --tone-cyan
  "#d5d8dd", // white — light gray (--input)
  "#62666e", // brightBlack — --muted-foreground (dim text)
  "#c13434", // brightRed
  "#1f9a5c", // brightGreen
  "#a97c2f", // brightYellow
  "#3f68c8", // brightBlue
  "#cb5fa4", // brightMagenta
  "#149aa6", // brightCyan
  "#f5f6f8", // brightWhite
]);

/** A palette is only usable when it fills all 16 slots. */
function isCompletePalette(colors: AnsiPalette | null | undefined): colors is AnsiPalette {
  return Array.isArray(colors) && colors.length === 16;
}

/**
 * Resolves the effective palette: a complete user override wins, otherwise the
 * palette follows the active light/dark theme.
 */
export function resolveAnsiPalette(
  override: AnsiPalette | null | undefined,
  dark: boolean,
): AnsiPalette {
  if (isCompletePalette(override)) return override;
  return dark ? DARK_ANSI : LIGHT_ANSI;
}
