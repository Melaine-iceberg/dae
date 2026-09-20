/**
 * Stable visual identity for Spaces (SKILL.md §4.2: "A Space should visually
 * communicate its identity"). Preset spaces get fixed accents; custom spaces
 * derive one from a hash of their id, so the color never changes between
 * sessions. Accents stay muted and confined to the icon tile to keep the
 * neutral theme coherent (§11).
 *
 * The six hues are theme tokens (--tone-*) rather than hand-written Tailwind
 * ramps: the ramp version needed a `dark:` variant on every entry, and the
 * one that was easiest to forget was the one that washed out. A token carries
 * both schemes, so a tone is declared once in App.css and used once here.
 */

interface SpaceAccent {
  tile: string;
  text: string;
}

const SPACE_ACCENTS: readonly SpaceAccent[] = [
  { tile: "bg-tone-blue/15", text: "text-tone-blue" },
  { tile: "bg-tone-violet/15", text: "text-tone-violet" },
  { tile: "bg-tone-emerald/15", text: "text-tone-emerald" },
  { tile: "bg-tone-amber/15", text: "text-tone-amber" },
  { tile: "bg-tone-rose/15", text: "text-tone-rose" },
  { tile: "bg-tone-cyan/15", text: "text-tone-cyan" },
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

export function getSpaceAccent(spaceId: string): SpaceAccent {
  const presetIndex = PRESET_ACCENTS[spaceId];
  const index = presetIndex ?? fnv1a(spaceId) % SPACE_ACCENTS.length;
  return SPACE_ACCENTS[index % SPACE_ACCENTS.length];
}
