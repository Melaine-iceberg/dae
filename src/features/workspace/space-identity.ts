/**
 * Stable visual identity for Spaces (SKILL.md §4.2: "A Space should visually
 * communicate its identity"). Preset spaces get fixed accents; custom spaces
 * derive one from a hash of their id, so the color never changes between
 * sessions. Accents stay muted and confined to the icon tile to keep the
 * neutral theme coherent (§11).
 */

interface SpaceAccent {
  tile: string;
  text: string;
}

const SPACE_ACCENTS: readonly SpaceAccent[] = [
  { tile: "bg-blue-500/15", text: "text-blue-600 dark:text-blue-400" },
  { tile: "bg-violet-500/15", text: "text-violet-600 dark:text-violet-400" },
  { tile: "bg-emerald-500/15", text: "text-emerald-600 dark:text-emerald-400" },
  { tile: "bg-amber-500/15", text: "text-amber-600 dark:text-amber-400" },
  { tile: "bg-rose-500/15", text: "text-rose-600 dark:text-rose-400" },
  { tile: "bg-cyan-500/15", text: "text-cyan-600 dark:text-cyan-400" },
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
