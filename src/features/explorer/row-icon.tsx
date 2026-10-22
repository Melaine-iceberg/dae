import { EntryIconFrame } from "./entry-badges";
import { EntryArtwork } from "./entry-artwork";
import type { ExtensionPresentation } from "./file-icons";
import type { DirectoryEntry } from "./types";

/** The row's icon cell (`--size-tile-list`), which every artwork below fills. */
const ROW_TILE_PX = 26;

/**
 * What a list or column row puts in its icon slot: the file's own frame, the
 * shell's icon, or the drawn type glyph.
 *
 * Shared because the two views ask the same question with different sizes, and
 * the answer is a three-way choice a row cannot make consistently twice. The
 * choice itself lives in `EntryArtwork`, which every other file surface also
 * uses.
 */
export function RowEntryIcon({
  entry,
  glyphSize,
  nativePixelSize,
  presentation,
}: {
  entry: DirectoryEntry;
  /** The glyph drawn inside the tile — the two views differ here. */
  glyphSize: number;
  nativePixelSize: number;
  presentation: ExtensionPresentation;
}) {
  return (
    <EntryIconFrame entry={entry}>
      <EntryArtwork
        className="size-tile-list shrink-0 rounded-xs"
        displaySize={ROW_TILE_PX}
        entry={entry}
        fallback={<presentation.icon size={glyphSize} />}
        pixelSize={nativePixelSize}
      />
    </EntryIconFrame>
  );
}
