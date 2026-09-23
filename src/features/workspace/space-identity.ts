/**
 * Stable visual identity for Spaces (SKILL.md §4.2: "A Space should visually
 * communicate its identity"). Preset spaces get fixed hues; custom spaces
 * derive one from a hash of their id, so the color never changes between
 * sessions.
 *
 * The hue is confined to the glyph. It used to also paint a 15% wash behind
 * it, which turned a sidebar row and an overview card into six little colour
 * chips — the chip plate is now always `bg-secondary`, and identity lives in
 * one stroke of colour on one icon. That is the whole rule for colour in this
 * app: a plate is neutral, a glyph may be coloured when the colour carries
 * meaning or identity.
 *
 * The six hues are theme tokens (--tone-*) rather than hand-written Tailwind
 * ramps: the ramp version needed a `dark:` variant on every entry, and the
 * one that was easiest to forget was the one that washed out. A token carries
 * both schemes, so a tone is declared once in App.css and used once here.
 */

const SPACE_ACCENTS: readonly string[] = [
  "text-tone-blue",
  "text-tone-violet",
  "text-tone-emerald",
  "text-tone-amber",
  "text-tone-rose",
  "text-tone-cyan",
];

const PRESET_ACCENTS: Record<string, number> = {
  work: 0,
  personal: 1,
  shared: 2,
  archive: 3,
};

function fnv1a(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/** The Tailwind text class carrying a Space's hue, for its glyph only. */
export function getSpaceAccentClass(spaceId: string): string {
  const presetIndex = PRESET_ACCENTS[spaceId];
  const index = presetIndex ?? fnv1a(spaceId) % SPACE_ACCENTS.length;
  return SPACE_ACCENTS[index % SPACE_ACCENTS.length];
}
