/**
 * Curated 16-color ANSI palettes for the integrated terminal.
 *
 * xterm's built-in ANSI colors are saturated primaries designed for a pure
 * black background; on the app's surfaces (`--card` is `#101111` dark,
 * `#ffffff` light) they are harsh and, in the light theme, several are outright
 * unreadable. These palettes are sampled from the house color tokens in
 * `App.css` and tuned per theme so `ls --color`, git diffs and friends are
 * legible in both modes.
 *
 * Sampled, not referenced — which matters most at the accent seam. ANSI blue
 * means *blue* the way `--success` means *success*, so a path `ls --color`
 * painted as a symlink keeps that meaning whatever hue the user has set the
 * window to; the accent retints the shell and not the terminal's colour
 * grammar. The default accent happens to be a blue as well, which is a
 * coincidence and not a relationship: the test is that setting a red or a
 * green system accent leaves this table untouched. Only the terminal's own
 * cursor/selection reads `--primary` from CSS and follows the accent seam live.
 * This table is plain hex anyway: xterm is handed values, not CSS, and cannot
 * follow a variable.
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

/** Dark, tuned to `--card: #101111` / `--foreground: #f9f9f9`. Measured against
 *  that panel, every slot but `black` clears 6.7:1 — `brightBlack` is the floor
 *  — and 6.1:1 on `--accent` #1b1c1e, the lightest surface xterm text ever
 *  lands on; `white` is 18:1 and `brightWhite` 18.9:1.
 *
 *  The three neutral slots are verbatim copies of `--background`,
 *  `--foreground` and `--muted-foreground`, so retuning the scheme means
 *  bringing them along by hand — nothing here can read a variable, and nothing
 *  here fails loudly when one is left behind. `black` had been: it still held
 *  the void's PREVIOUS value (#1a1917, L* 8.8) after the canvas moved to
 *  #07080a, which put it 4.3 L* ABOVE the panel it is meant to sit below — so a
 *  box drawn in ANSI black lit up instead of recessing. All three are the
 *  current tokens again.
 *
 *  `black` at 1.06:1 is not a contrast failure, it is the ceiling of the move:
 *  a shadow is ink, and this panel is nearly ink already — pure black against
 *  #101111 measures 1.11:1, so nothing on this scheme can recess further than
 *  that. Slot 0 is therefore a FILL tone (a reverse-video block, the inside of
 *  a box) and slot 8 is what dim text uses. */
export const DARK_ANSI: AnsiPalette = Object.freeze([
  "#07080a", // black — --background: one step BELOW the panel, so an ANSI-black box recesses rather than lighting up (1.06:1 — a fill tone; see above)
  "#ff8288", // red — --destructive
  "#63d398", // green — --success
  "#e8a33d", // yellow — warm amber (--warning)
  "#8fb2ff", // blue — a real blue: ANSI blue means blue (see the header), never a retint of the accent
  "#c792ea", // magenta
  "#4ec2ea", // cyan — --tone-cyan
  "#f9f9f9", // white — --foreground (17.96:1 on the panel)
  "#9c9c9d", // brightBlack — --muted-foreground (6.77:1 on the panel, 6.11:1 on the selection fill)
  "#ffa3a7", // brightRed
  "#8ce0b3", // brightGreen
  "#ffc868", // brightYellow — warm amber pastel
  "#a8c7ff", // brightBlue
  "#f08fce", // brightMagenta
  "#7fd2f0", // brightCyan
  "#ffffff", // brightWhite
]);

/** Light, tuned to `--card: #ffffff`. The panel is paper here, so the neutral
 *  ladder runs the other way: the darkest ink is the text and the lightest
 *  tone is what a plate is filled with. The four neutral slots are therefore
 *  the cool tokens, slot for slot against the dark table above:
 *
 *    slot 0   #101216  --foreground        the ink (18.75:1 on the paper)
 *    slot 7   #d2d7de  --input             the paper step (1.45:1 — a FILL)
 *    slot 8   #5b6169  --muted-foreground  dim text (6.25:1 / 5.19:1 on selection)
 *    slot 15  #eef0f3  --background        the canvas below the card (1.14:1)
 *
 *  The two extremes swap roles with the scheme rather than changing value: in
 *  dark, slot 0 is the void one step BELOW the panel and slot 15 is the one
 *  tone above it; in light, slot 0 is the one ink below the panel's paper and
 *  slot 15 is the canvas the paper lies on. That is the same physical
 *  relationship read in daylight, which is why slot 0 is a readable 18.75:1
 *  here and a 1.06:1 fill there — the extremes are the two ends of one ladder,
 *  and which end carries text is what flips.
 *
 *  Slots 7 and 15 are FILL tones, not text: white-on-white is the one thing a
 *  light terminal cannot print, and every light scheme answers it the same way
 *  — `white` is the plate a reverse-video block sits on. This table used to
 *  paint the warm set (#37352f / #d8d4cb / #5f5c55 / #f6f5f1) against a cool
 *  canvas; all four followed the retune.
 *
 *  The hue slots below are untouched by that retune and are the table's weak
 *  end — measured on the paper: yellow 3.07:1, brightYellow 2.27:1, brightCyan
 *  2.94:1, brightGreen 3.60:1, brightMagenta 3.68:1, brightBlue 3.85:1, cyan
 *  4.13:1. Only the reds (5.6:1), green (4.97:1), blue (5.22:1) and magenta
 *  (4.99:1) clear 4.5:1. Retuning those is a hue decision, not a value that
 *  can be read off a token, so it is recorded here rather than guessed at. */
export const LIGHT_ANSI: AnsiPalette = Object.freeze([
  "#101216", // black — --foreground: the ink, and the darkest thing on the paper (18.75:1)
  "#c22f2f", // red — --destructive
  "#17804a", // green — --success
  "#c78600", // yellow — the folder gold, kept at the dark rung so it reads on white
  "#3f68c8", // blue — a real blue (see the header); never a retint of the accent
  "#b8438a", // magenta
  "#0a86b4", // cyan — --tone-cyan
  "#d2d7de", // white — --input: the paper step (1.45:1 — the fill a block sits on)
  "#5b6169", // brightBlack — --muted-foreground (6.25:1 on the paper, 5.19:1 on the selection)
  "#c13434", // brightRed
  "#1f9a5c", // brightGreen
  "#e89d01", // brightYellow
  "#5b7fd6", // brightBlue
  "#cb5fa4", // brightMagenta
  "#22a3c8", // brightCyan
  "#eef0f3", // brightWhite — --background: the canvas the card lies on (1.14:1 — a fill)
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
