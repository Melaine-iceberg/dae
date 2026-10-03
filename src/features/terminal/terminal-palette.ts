/**
 * Curated 16-color ANSI palettes for the integrated terminal.
 *
 * xterm's built-in ANSI colors are saturated primaries designed for a pure
 * black background; on the app's surfaces (`--card` is `#1d1f22` dark,
 * `#ffffff` light) they are harsh and, in the light theme, several are outright
 * unreadable. These palettes are sampled from the house color tokens in
 * `App.css` and tuned per theme so `ls --color`, git diffs and friends are
 * legible in both modes.
 *
 * Sampled, not referenced — which matters most at the accent seam. ANSI blue
 * means *blue* the way `--success` means *success*, so a path `ls --color`
 * painted as a symlink keeps that meaning whatever hue the user has set the
 * window to; the accent retints the shell and not the terminal's colour
 * grammar. The shipped accent is teal, which is NOT a blue — so the blue slot
 * holds a real blue and the accent no longer appears in this table at all
 * (the terminal's own cursor/selection reads `--primary` from CSS and follows
 * the accent seam live). This table is plain hex anyway: xterm is handed
 * values, not CSS, and cannot follow a variable.
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

/** Dark, tuned to `--card: #1d1f22` / `--foreground: #e5e6e8`. Every slot but
 *  `black` clears ≥5.5:1 on the panel, and ≥4.5:1 on the lightest tone text
 *  ever lands on there. */
export const DARK_ANSI: AnsiPalette = Object.freeze([
  "#131417", // black — the canvas tone: one step BELOW the panel, so a box drawn in ANSI black recesses instead of lighting up
  "#ff8288", // red — --destructive
  "#63d398", // green — --success
  "#e8a33d", // yellow — warm amber (--warning)
  "#8fb2ff", // blue — a real blue: ANSI blue means blue (see the header), not the teal accent
  "#c792ea", // magenta
  "#4ec2ea", // cyan — --tone-cyan
  "#e5e6e8", // white — --foreground
  "#9ba0a9", // brightBlack — --muted-foreground (dim text stays legible)
  "#ffa3a7", // brightRed
  "#8ce0b3", // brightGreen
  "#ffc868", // brightYellow — warm amber pastel
  "#a8c7ff", // brightBlue
  "#f08fce", // brightMagenta
  "#7fd2f0", // brightCyan
  "#ffffff", // brightWhite
]);

/** Light, tuned to `--card: #ffffff` / `--foreground: #282a30`. */
export const LIGHT_ANSI: AnsiPalette = Object.freeze([
  "#282a30", // black — soft near-black (--foreground), not pure #000
  "#c22f2f", // red — --destructive
  "#17804a", // green — --success
  "#8a6318", // yellow — darkened amber so it reads on white
  "#3f68c8", // blue — a real blue (see the header); not the teal accent
  "#b8438a", // magenta
  "#0a86b4", // cyan — --tone-cyan
  "#d5d8dd", // white — light gray (--input)
  "#62666e", // brightBlack — --muted-foreground (dim text)
  "#c13434", // brightRed
  "#1f9a5c", // brightGreen
  "#a97c2f", // brightYellow
  "#5b7fd6", // brightBlue
  "#cb5fa4", // brightMagenta
  "#22a3c8", // brightCyan
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
