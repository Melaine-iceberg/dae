/**
 * Curated 16-color ANSI palettes for the integrated terminal.
 *
 * xterm's built-in ANSI colors are saturated primaries designed for a pure
 * black background; on the app's graphite surfaces (`--card` is `#16191f` dark,
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

/** Graphite dark, tuned to `--card: #16191f` / `--foreground: #e7e9ee`. */
export const DARK_ANSI: AnsiPalette = Object.freeze([
  "#2e3540", // black — the border tone, never pure #000 (too harsh on graphite)
  "#ff6b72", // red — --destructive
  "#63d398", // green — --icon-sheet
  "#e8a33d", // yellow — warm amber (--chart-4)
  "#7498ff", // blue — --primary, already light enough for the dark island
  "#c792ea", // magenta
  "#5acfd9", // cyan — --icon-image
  "#e7e9ee", // white — --foreground
  "#a2a9b6", // brightBlack — --muted-foreground (dim text stays legible)
  "#ff8a8a", // brightRed — --icon-pdf
  "#8ce0b3", // brightGreen
  "#ffc868", // brightYellow — warm amber pastel
  "#8fb2ff", // brightBlue — --icon-doc
  "#f08fce", // brightMagenta — --icon-video
  "#8ce0e6", // brightCyan
  "#f2f3f5", // brightWhite
]);

/** Graphite light, tuned to `--card: #ffffff` / `--foreground: #171a21`. */
export const LIGHT_ANSI: AnsiPalette = Object.freeze([
  "#171a21", // black — soft near-black, not pure #000
  "#b93030", // red — --destructive
  "#17804a", // green — --icon-sheet
  "#8a6318", // yellow — darkened amber so it reads on white
  "#2a55ce", // blue — --primary
  "#b8438a", // magenta — --icon-video
  "#0d848c", // cyan — --icon-image
  "#bdc4d1", // white — light gray (--input)
  "#4e5665", // brightBlack — --muted-foreground (dim text)
  "#c13434", // brightRed — --icon-pdf
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
