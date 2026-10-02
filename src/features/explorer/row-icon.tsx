import { useAtomValue } from "jotai";

import { EntryIconFrame } from "./entry-badges";
import type { ExtensionPresentation } from "./file-icons";
import { TypeIconTile } from "./icon-tile";
import { NativeIconImage, useNativeIconFor } from "./native-icon";
import { rowThumbnailsAtom } from "./preferences";
import { ThumbnailImage, isThumbnailSupported } from "./thumbnail";
import type { DirectoryEntry } from "./types";

/** The row's icon cell (`--size-tile-list`), which every artwork below fills. */
const ROW_TILE_PX = 22;

/**
 * What a list or column row puts in its icon slot: the file's own frame, the
 * shell's icon, or the drawn type glyph.
 *
 * Shared because the two views ask the same question with different sizes, and
 * the answer is a three-way choice a row cannot make consistently twice — the
 * grid already keeps its own version of this branch, at a size where a
 * thumbnail is legible enough to be the default.
 *
 * A thumbnail outranks the shell icon rather than the reverse: when `"system"`
 * icons are on, the user asked for the file's real artwork, and a first frame
 * says more about a clip or a PDF than its handler's registered icon does. Both
 * fall back to the glyph, which is what keeps a row honest about the file type
 * when neither the protocol nor the shell has an answer.
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
  const rowThumbnails = useAtomValue(rowThumbnailsAtom);
  const showNativeIcon = useNativeIconFor(entry);
  const glyph = (
    <TypeIconTile className="size-tile-list" iconSize={glyphSize} presentation={presentation} />
  );

  if (rowThumbnails && isThumbnailSupported(entry)) {
    return (
      <EntryIconFrame entry={entry}>
        <ThumbnailImage
          className="size-tile-list shrink-0 rounded-xs"
          displaySize={ROW_TILE_PX}
          entry={entry}
          fallback={glyph}
          // The hairline plate a grid cell puts around its picture would
          // outline every icon in a column of rows here.
          plate={false}
        />
      </EntryIconFrame>
    );
  }

  return (
    <EntryIconFrame entry={entry}>
      {showNativeIcon ? (
        <NativeIconImage
          className="shrink-0"
          entry={entry}
          fallback={glyph}
          pixelSize={nativePixelSize}
        />
      ) : (
        glyph
      )}
    </EntryIconFrame>
  );
}
