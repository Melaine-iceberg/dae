/**
 * Curated 16-color ANSI palettes for the integrated terminal.
 *
 * xterm's built-in ANSI colors are saturated primaries designed for a pure
 * black background; on the app's graphite surfaces (`--card` is `#1d1f27` dark,
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

/** Graphite dark, tuned to `--card: #1d1f27` / `--foreground: #dfe1e5`. */
export const DARK_ANSI: AnsiPalette = Object.freeze([
  "#31343f", // black — the border tone, never pure #000 (too harsh on graphite)
  "#e55765", // red — --destructive
  "#6fd69d", // green — --icon-sheet
  "#e8a33d", // yellow — warm amber (--chart-4)
  "#6aa5f5", // blue — lightened --primary for contrast on the dark island
  "#c792ea", // magenta
  "#66d2da", // cyan — --icon-image
  "#dfe1e5", // white — --foreground
  "#9da0a8", // brightBlack — --muted-foreground (dim text stays legible)
  "#ff8f8f", // brightRed — --icon-pdf
  "#8ce0b3", // brightGreen
  "#ffc868", // brightYellow — --folder (dark)
  "#93b6ff", // brightBlue — --icon-doc
  "#f295d1", // brightMagenta — --icon-video
  "#8ce0e6", // brightCyan
  "#f2f3f5", // brightWhite
]);

/** Graphite light, tuned to `--card: #ffffff` / `--foreground: #1f2328`. */
export const LIGHT_ANSI: AnsiPalette = Object.freeze([
  "#24292f", // black — soft near-black, not pure #000
  "#d5373a", // red — --destructive
  "#2e9e63", // green — --icon-sheet
  "#a3762a", // yellow — darkened amber so it reads on white
  "#3574f0", // blue — --primary
  "#c94fa0", // magenta — --icon-video
  "#159aa3", // cyan — --icon-image
  "#c4c8ce", // white — light gray (--input)
  "#6c707e", // brightBlack — --muted-foreground (dim text)
  "#df4a52", // brightRed
  "#4caf7d", // brightGreen
  "#c08a2e", // brightYellow
  "#5b93f5", // brightBlue
  "#d96fb4", // brightMagenta
  "#2bb3bd", // brightCyan
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
