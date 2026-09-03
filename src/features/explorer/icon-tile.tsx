import type { CSSProperties } from "react";

import { cn } from "@/lib/utils";

import type { ExtensionPresentation } from "./file-icons";

/**
 * Type icon tile: the presentation glyph seated on a soft squircle. Folders
 * get the blue gradient hero tile (`.tile-folder` in App.css) with a white
 * fill glyph; typed files sit on a 13% tint of their category color; untoned
 * kinds (plain files, symlinks, others) fall back to a neutral muted tile.
 * Native shell icons and thumbnails never use tiles — they are rich enough
 * on their own.
 */

/** "text-icon-pdf" → "--icon-pdf"; the folder tone maps to the gradient tile. */
function toneVarName(tone: string | undefined): string | null {
  if (!tone || tone === "text-folder") return null;
  return `--${tone.slice("text-".length)}`;
}

/** Inline style for a 13% type-color tint; shared by tiles and card chips. */
export function tintStyle(varName: string): CSSProperties {
  return {
    backgroundColor: `color-mix(in oklab, var(${varName}) 13%, transparent)`,
    color: `var(${varName})`,
  };
}

export function TypeIconTile({
  className,
  iconClassName,
  iconSize,
  pop = false,
  presentation,
}: {
  /** Tile geometry: size + corner radius (e.g. "size-11 rounded-[13px]"). */
  className?: string;
  iconClassName?: string;
  iconSize: number;
  /** Spring up under row/cell hover (the house entry-icon-pop curve). */
  pop?: boolean;
  presentation: ExtensionPresentation;
}) {
  const Icon = presentation.icon;
  const isFolder = presentation.tone === "text-folder";
  const varName = toneVarName(presentation.tone);

  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex shrink-0 items-center justify-center",
        pop && "entry-icon-pop",
        isFolder ? "tile-folder" : !varName && "bg-muted text-muted-foreground",
        className,
      )}
      style={varName ? tintStyle(varName) : undefined}
    >
      <Icon
        className={iconClassName}
        size={iconSize}
        weight={isFolder ? "fill" : undefined}
      />
    </span>
  );
}
