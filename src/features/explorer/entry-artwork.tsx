import { useAtomValue } from "jotai";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

import { NativeIconImage, useNativeIconFor } from "./native-icon";
import { rowThumbnailsAtom } from "./preferences";
import { ThumbnailImage, isThumbnailSupported } from "./thumbnail";
import type { ArtworkEntry } from "./types";

/** When a surface is allowed to draw the file's own frame. */
export type ArtworkThumbnailMode =
  /** Whatever the list-row preference says — the list, column, recents,
   *  overview, content-search and trash rows. */
  | "auto"
  /** Large artwork where a first frame is always legible — the grid. */
  | "always"
  /** Chrome-sized slots where a thumbnail would only be a smudge. */
  | "never";

/**
 * One file entry's artwork: its own first frame, the operating system's icon,
 * or the drawn type glyph, in that order.
 *
 * The three sources used to be branched at each call site, which is why the
 * explorer honoured the icon style while recents, search, the trash, properties
 * and the conflict dialog kept drawing glyphs long after the protocols existed.
 * Every surface that has an `ArtworkEntry` now asks here and gets the same
 * answer; only the cell size and the thumbnail policy differ.
 *
 * A thumbnail outranks the shell icon: when system icons are on, the user asked
 * for the file's real artwork, and a first frame says more about a clip or a
 * PDF than its handler's registered icon does. Both fall back to the glyph, so
 * a missing protocol response never leaves a hole in the list.
 */
export function EntryArtwork({
  className,
  displaySize,
  entry,
  fallback,
  pixelSize = displaySize,
  plate = false,
  thumbnail = "auto",
}: {
  /** The sized cell the artwork fills. */
  className?: string;
  /** The CSS pixels the thumbnail covers, so the request asks for the device
   *  pixels actually painted. */
  displaySize: number;
  entry: ArtworkEntry | null | undefined;
  fallback: ReactNode;
  /** The device-pixel-independent size of a native/glyph icon when it differs
   *  from the cell — a 20px shell icon in the 26px row tile. */
  pixelSize?: number;
  plate?: boolean;
  thumbnail?: ArtworkThumbnailMode;
}) {
  const rowThumbnails = useAtomValue(rowThumbnailsAtom);
  const showNativeIcon = useNativeIconFor(entry);
  const showThumbnail =
    entry != null &&
    isThumbnailSupported(entry) &&
    (thumbnail === "always" || (thumbnail === "auto" && rowThumbnails));

  if (entry != null && showThumbnail) {
    return (
      <ThumbnailImage
        className={className}
        displaySize={displaySize}
        entry={entry}
        fallback={fallback}
        plate={plate}
      />
    );
  }

  if (entry != null && showNativeIcon) {
    return (
      <NativeIconImage
        className={className}
        entry={entry}
        fallback={fallback}
        pixelSize={pixelSize}
      />
    );
  }

  return (
    <span className={cn("inline-flex shrink-0 items-center justify-center", className)}>
      {fallback}
    </span>
  );
}
