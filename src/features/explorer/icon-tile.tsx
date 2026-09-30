import { cn } from "@/lib/utils";

import type { ExtensionPresentation } from "./file-icons";

/**
 * Entry icon frame: the type glyph seated in a fixed square cell so it stays
 * aligned with native shell icons and thumbnails in the same view.
 *
 * The cell is pure layout — there is no tinted plate behind the glyph. A type
 * glyph is a 16px silhouette that already carries its category colour, so a
 * coloured plate under it would only add a second, competing shape.
 */

export function TypeIconTile({
  className,
  iconClassName,
  iconSize,
  presentation,
}: {
  /** Cell geometry: a fixed square from the `--size-tile-*` scale. */
  className?: string;
  iconClassName?: string;
  iconSize: number;
  presentation: ExtensionPresentation;
}) {
  const Icon = presentation.icon;

  return (
    <span aria-hidden="true" className={cn("flex shrink-0 items-center justify-center", className)}>
      <Icon className={iconClassName} size={iconSize} />
    </span>
  );
}
